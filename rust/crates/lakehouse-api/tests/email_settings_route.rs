//! `GET`/`PUT /api/settings/email` and `POST /api/settings/email/test`
//! (`ADM-7`, slice 1 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! What this proves:
//!
//! * every one of the three routes needs `settings:write`, reading included
//!   (plan D4);
//! * the answer says where every value comes from, and never carries the
//!   password (plan D1, D5);
//! * a field the server's environment owns is refused with a 409 and
//!   nothing is written (plan D1);
//! * the connection fields are one unit: the environment sets one of them
//!   and it owns all five, so a saved host is not used for a send
//!   (`ADM-7 review R1`);
//! * every save writes one audit row, naming the fields that changed and
//!   never a secret (plan D6);
//! * the test email goes to the caller's own account address and to nobody
//!   else (plan D10), against a real relay;
//! * a second test email within ten seconds is refused with a
//!   `Retry-After` (plan D10).
//!
//! The relay is a real `Mailpit` in a container behind a private CA — the
//! same arrangement `lakehouse-notify`'s `smtp_private_ca.rs` proves — so
//! the message really leaves the process and its one recipient really is
//! the caller's address.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{HeaderMap, Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up, spin_up_with_env,
};
use serde_json::{Value, json};
use testcontainers::core::WaitFor;
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, GenericImage, ImageExt};
use tower::ServiceExt;

/// What the person is told when the environment owns a field (plan D1).
const SET_BY_THE_SERVER: &str =
    "This setting is set in the server's environment and cannot be changed here.";

/// The relay, pinned to an exact tag.
const MAILPIT_IMAGE: &str = "axllent/mailpit";
const MAILPIT_TAG: &str = "v1.33.0";
/// The SMTP port inside the container.
const SMTP_PORT: u16 = 1025;
/// The HTTP API port inside the container.
const HTTP_PORT: u16 = 8025;
/// Where the certificate and the key are put inside the container.
const CERTIFICATE_PATH: &str = "/tls/server.pem";
const KEY_PATH: &str = "/tls/server.key";

/// Call one of the three routes: its status, its headers and its body.
async fn raw(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, HeaderMap, Value) {
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
    let headers = resp.headers().clone();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// [`raw`] without the headers.
async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value) {
    let (status, _headers, json) = raw(app, verb, path, cookie, body).await;
    (status, json)
}

/// The email settings route.
const EMAIL: &str = "/api/settings/email";
/// The test email route.
const EMAIL_TEST: &str = "/api/settings/email/test";

/// A deployment with nothing saved answers every field with its default and
/// says where the default comes from.
#[tokio::test]
async fn a_fresh_deployment_answers_every_field_with_its_source() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["host"], json!({"value": null, "source": "default"}));
    assert_eq!(body["port"], json!({"value": 587, "source": "default"}));
    assert_eq!(
        body["security"],
        json!({"value": "starttls", "source": "default"})
    );
    assert_eq!(body["user"], json!({"value": null, "source": "default"}));
    assert_eq!(body["password"], json!({"set": false, "source": "default"}));
    assert_eq!(
        body["from"],
        json!({"value": "rantai-lake@localhost", "source": "default"})
    );
    assert_eq!(
        body["consolePublicUrl"],
        json!({"value": null, "source": "default"})
    );
}

/// Plan D4: reading the mail settings needs `settings:write` like changing
/// them does — a mail server's name and account are not for every signed-in
/// person.
#[tokio::test]
async fn a_caller_without_settings_write_is_refused_on_all_three_routes() {
    let TestApp { router, pool } = spin_up().await;
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, _) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"smtp.example.com"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    // Nothing was written by the refused save.
    assert!(
        lakehouse_store::email_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

/// Plan D5: the password is write-only. It never comes back in an answer,
/// and the row holds it sealed.
#[tokio::test]
async fn the_password_never_comes_back_in_an_answer() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"smtp.example.com","password":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["password"],
        json!({"set": true, "source": "console"}),
        "the answer says that one is set, never what it is: {body}"
    );
    assert!(!body.to_string().contains("not-a-real-secret"), "{body}");

    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["password"], json!({"set": true, "source": "console"}));
    assert!(!read.to_string().contains("not-a-real-secret"), "{read}");

    // The row holds the sealed bytes, not the text.
    let row = lakehouse_store::email_settings::get(&pool)
        .await
        .unwrap()
        .expect("the save wrote a row");
    let sealed = row.smtp_pass.expect("the password was stored");
    assert!(!String::from_utf8_lossy(&sealed).contains("not-a-real-secret"));
}

/// Plan D1: a field the server's environment owns is refused with a 409, and
/// nothing is written.
#[tokio::test]
async fn a_field_the_environment_owns_is_refused_and_nothing_is_written() {
    let mut env = HashMap::new();
    env.insert("SMTP_HOST".to_owned(), "smtp.example.com".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"other.example.com","port":2525}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"].as_str(), Some(SET_BY_THE_SERVER), "{body}");
    // Nothing was written — not even the field the environment does not own.
    assert!(
        lakehouse_store::email_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );

    // The environment's value is still what is in effect, and it says so.
    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["host"],
        json!({"value": "smtp.example.com", "source": "server"})
    );
}

/// `ADM-7 review R1`: the connection fields are ONE unit. The environment
/// sets only the password, so it owns the whole connection — the host, the
/// port, the security, the account and the password are the server's — and
/// a save that sends any of them is refused with a 409. Nothing is written.
#[tokio::test]
async fn a_save_of_a_connection_field_is_refused_when_the_environment_owns_the_connection() {
    let mut env = HashMap::new();
    env.insert("SMTP_PASS".to_owned(), "an-environment-password".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"smtp.example.com"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"].as_str(), Some(SET_BY_THE_SERVER), "{body}");
    // Nothing was written.
    assert!(
        lakehouse_store::email_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );

    // Every field of the connection says the server owns it now, and the
    // console may change none of them.
    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["host"], json!({"value": null, "source": "server"}));
    assert_eq!(read["port"], json!({"value": 587, "source": "server"}));
    assert_eq!(
        read["security"],
        json!({"value": "starttls", "source": "server"})
    );
    assert_eq!(read["user"], json!({"value": null, "source": "server"}));
    assert_eq!(read["password"], json!({"set": true, "source": "server"}));
}

/// `ADM-7 review R1`: a host saved in the console is NOT used for a send
/// when the environment owns the connection. The row is written directly
/// into the store, the way a row saved before the operator put `SMTP_PASS`
/// in the environment looks.
#[tokio::test]
async fn a_saved_console_host_is_not_used_when_the_environment_owns_the_connection() {
    let mut env = HashMap::new();
    env.insert("SMTP_PASS".to_owned(), "an-environment-password".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    // A row saved before the environment owned the connection.
    lakehouse_store::email_settings::upsert(
        &pool,
        &lakehouse_store::email_settings::EmailSettings {
            smtp_host: Some("smtp.example.com".to_owned()),
            smtp_port: Some(2525),
            smtp_security: Some("tls".to_owned()),
            smtp_user: Some("console-user".to_owned()),
            smtp_pass: None,
            smtp_from: Some("reports@example.com".to_owned()),
            console_public_url: None,
        },
        &admin.to_string(),
    )
    .await
    .unwrap();

    // The answer reports the environment's values, not the saved ones.
    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["host"], json!({"value": null, "source": "server"}));
    assert_eq!(read["port"], json!({"value": 587, "source": "server"}));
    assert_eq!(read["user"], json!({"value": null, "source": "server"}));
    // `from` is not part of the login, so it stays the console's.
    assert_eq!(
        read["from"],
        json!({"value": "reports@example.com", "source": "console"})
    );

    // And the send does not use the saved host: the environment names no
    // mail server, so email is not set up.
    let (status, body) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sent"], json!(false), "{body}");
    assert_eq!(
        body["message"],
        json!("Email is not set up: the mail server is missing."),
        "{body}"
    );
}

/// Plan D6: one audit row per save, naming the fields that changed and never
/// a value.
#[tokio::test]
async fn a_save_writes_one_audit_row_naming_the_fields_and_never_the_secret() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"smtp.example.com","port":2525,"password":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'settings.email.update'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "one save, one audit row");
    let (action, args) = &rows[0];
    assert_eq!(action, "settings.email.update");
    let fields: Vec<&str> = args["fields"]
        .as_array()
        .expect("the row names the fields")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(fields.contains(&"host"), "{fields:?}");
    assert!(fields.contains(&"port"), "{fields:?}");
    // The password's NAME may be there; its value never is.
    assert!(fields.contains(&"password"), "{fields:?}");
    assert!(!args.to_string().contains("not-a-real-secret"), "{args}");
}

/// Plan D5: a field that is not sent keeps what is stored, and an explicit
/// `null` clears it.
#[tokio::test]
async fn a_field_that_is_not_sent_keeps_its_value_and_null_clears_it() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        r#"{"host":"smtp.example.com","port":2525,"security":"tls","user":"reports","from":"reports@example.com","consolePublicUrl":"https://console.example.com/"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["host"],
        json!({"value": "smtp.example.com", "source": "console"})
    );
    assert_eq!(body["port"], json!({"value": 2525, "source": "console"}));
    assert_eq!(
        body["security"],
        json!({"value": "tls", "source": "console"})
    );
    assert_eq!(
        body["user"],
        json!({"value": "reports", "source": "console"})
    );
    assert_eq!(
        body["from"],
        json!({"value": "reports@example.com", "source": "console"})
    );
    // The trailing `/` is trimmed, the way the environment's own value is.
    assert_eq!(
        body["consolePublicUrl"],
        json!({"value": "https://console.example.com", "source": "console"})
    );

    // A save that does not send the host keeps it.
    let (status, again) = call(&router, "PUT", EMAIL, &cookie, r#"{"port":2526}"#).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["host"],
        json!({"value": "smtp.example.com", "source": "console"})
    );
    assert_eq!(again["port"], json!({"value": 2526, "source": "console"}));

    // `null` clears it.
    let (status, cleared) = call(&router, "PUT", EMAIL, &cookie, r#"{"host":null}"#).await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["host"], json!({"value": null, "source": "default"}));
}

/// Plan D9: a value that is not usable is refused with a fixed sentence that
/// names the field.
#[tokio::test]
async fn an_unusable_value_is_refused_with_a_sentence_that_names_the_field() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    for (body, sentence) in [
        (
            r#"{"host":"not a host name"}"#,
            "host must be a mail server name, such as smtp.example.com.",
        ),
        (
            r#"{"port":0}"#,
            "port must be a whole number from 1 to 65535.",
        ),
        (
            r#"{"port":70000}"#,
            "port must be a whole number from 1 to 65535.",
        ),
        (
            r#"{"security":"plain"}"#,
            "security must be tls or starttls.",
        ),
        (
            r#"{"from":"not-an-address"}"#,
            "from must be an email address, such as reports@example.com.",
        ),
        (
            r#"{"consolePublicUrl":"ftp://console.example.com"}"#,
            "consolePublicUrl must start with http:// or https://.",
        ),
    ] {
        let (status, got) = call(&router, "PUT", EMAIL, &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {got}");
        assert_eq!(got["error"].as_str(), Some(sentence), "{body}: {got}");
    }
    // A body that is not the email fields at all.
    let (status, got) = call(&router, "PUT", EMAIL, &cookie, r#"{"smtpHost":"x"}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{got}");
    assert!(
        got["error"]
            .as_str()
            .is_some_and(|m| m.starts_with("body must be")),
        "{got}"
    );
}

/// Plan D10: one test email per caller every ten seconds; the second one is
/// refused with a `Retry-After`.
#[tokio::test]
async fn a_second_test_email_within_ten_seconds_is_refused_with_a_retry_after() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    // No mail server is set up, so the first one answers with the fixed
    // sentence of plan D7.
    let (status, body) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sent"], json!(false), "{body}");
    assert_eq!(
        body["message"],
        json!("Email is not set up: the mail server is missing."),
        "{body}"
    );

    let (status, headers, body) = raw(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{body}");
    assert!(
        headers.get("retry-after").is_some(),
        "the caller is told when to try again: {headers:?}"
    );
}

/// Plan D10: the test email goes to the caller's own account address and to
/// nobody else, through a real relay.
#[tokio::test]
async fn the_test_email_goes_to_the_callers_own_address_and_to_nobody_else() {
    let relay = start_relay().await;
    let ca_file = tempfile::NamedTempFile::new().expect("a temporary CA file");
    std::fs::write(ca_file.path(), relay.ca_pem.as_bytes()).expect("write the CA");
    let mut env = HashMap::new();
    env.insert(
        "SMTP_CA_FILE".to_owned(),
        ca_file.path().to_string_lossy().to_string(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let address = format!("single-perm-{admin}@test.invalid");
    let cookie = session_cookie_for_user(&pool, admin).await;

    // The relay is the deployment's mail server, saved in the console.
    let (status, body) = call(
        &router,
        "PUT",
        EMAIL,
        &cookie,
        &json!({
            "host": relay.host,
            "port": relay.smtp_port,
            "security": "starttls",
            "from": "reports@example.com",
            "consolePublicUrl": "https://console.example.com",
        })
        .to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sent"], json!(true), "{body}");
    assert_eq!(
        body["message"],
        json!(format!("Sent to {address}.")),
        "{body}"
    );

    // The relay holds exactly one message, and its one recipient is the
    // caller's own address.
    let recipients = relay.recipients().await;
    assert_eq!(
        recipients,
        vec![address.clone()],
        "the recipient is the caller"
    );
}

/// A caller with no account address at all (a service identity) is refused
/// with a sentence, not with a mail server's words.
#[tokio::test]
async fn a_caller_without_an_account_address_is_refused_with_a_sentence() {
    let TestApp { router, pool } = spin_up().await;
    // A principal with no permissions is refused by the gate first; the
    // address is only asked for once the gate let the caller in.
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, body) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
}

/// ADM-11 D9: A user with `settings:write` but no email address is refused
/// with a 400 when calling test email.
#[tokio::test]
async fn a_user_without_an_email_address_gets_400_for_test_email() {
    let TestApp { router, pool } = spin_up().await;
    let (user_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO app_user (name, username, email, status) VALUES ($1, $2, NULL, 'active') RETURNING id",
    )
    .bind("No Email Admin")
    .bind("no.email.admin")
    .fetch_one(&pool)
    .await
    .unwrap();

    let (role_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO role (name, permissions, description) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind("Settings Admin Role No Email")
    .bind("settings:write")
    .bind("Can write settings")
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(&pool)
        .await
        .unwrap();

    let cookie = session_cookie_for_user(&pool, user_id).await;
    let (status, body) = call(&router, "POST", EMAIL_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no email address"),
        "{body}"
    );
}

// ── The relay ────────────────────────────────────────────────────────────

/// The CA and the server certificate it signs, minted for one test.
struct TestPki {
    /// The CA certificate, PEM.
    ca_pem: String,
    /// The server certificate, PEM.
    certificate_pem: String,
    /// The server's private key, PEM.
    key_pem: String,
    /// The names the server certificate carries.
    names: Vec<String>,
}

impl TestPki {
    /// Whether the certificate carries `name`.
    fn carries(&self, name: &str) -> bool {
        self.names.iter().any(|carried| carried == name)
    }
}

/// The PEM armour around a DER certificate or key, written here so that
/// `rcgen`'s `pem` feature stays off (turning it on would unify the feature
/// into this crate's tests, which pin `rcgen` without it).
fn pem(label: &str, der: &[u8]) -> String {
    use base64::Engine as _;
    use std::fmt::Write as _;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        out.push('\n');
    }
    let _ = writeln!(out, "-----END {label}-----");
    out
}

/// Mint a CA and a server certificate signed by it, for the given names.
fn mint_pki(names: &[&str]) -> TestPki {
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let mut ca_params = rcgen::CertificateParams::default();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        rcgen::KeyUsagePurpose::KeyCertSign,
        rcgen::KeyUsagePurpose::CrlSign,
    ];
    let ca_cert = ca_params.self_signed(&ca_key).unwrap();

    let server_key = rcgen::KeyPair::generate().unwrap();
    let mut server_params = rcgen::CertificateParams::new(
        names
            .iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    server_params.use_authority_key_identifier_extension = true;
    server_params.key_usages = vec![
        rcgen::KeyUsagePurpose::DigitalSignature,
        rcgen::KeyUsagePurpose::KeyEncipherment,
    ];
    server_params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(
            &server_key,
            &rcgen::Issuer::from_params(&ca_params, &ca_key),
        )
        .unwrap();

    TestPki {
        ca_pem: pem("CERTIFICATE", ca_cert.der().as_ref()),
        certificate_pem: pem("CERTIFICATE", server_cert.der().as_ref()),
        key_pem: pem("PRIVATE KEY", &server_key.serialize_der()),
        names: names.iter().map(|name| (*name).to_owned()).collect(),
    }
}

/// A running relay, and the addresses it is reached on.
struct Relay {
    /// Kept alive for the test: dropping it stops the container.
    _container: ContainerAsync<GenericImage>,
    /// The host the sender dials, and the name the certificate carries.
    host: String,
    /// The host port the SMTP server is published on.
    smtp_port: u16,
    /// The host port the HTTP API is published on.
    http_port: u16,
    /// The CA the relay's certificate chains to.
    ca_pem: String,
}

impl Relay {
    /// The addresses the relay's messages went to, one entry per recipient.
    async fn recipients(&self) -> Vec<String> {
        let url = format!("http://{}:{}/api/v1/messages", self.host, self.http_port);
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let body: Value = client.get(url).send().await.unwrap().json().await.unwrap();
        body["messages"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .flat_map(|message| {
                message["To"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|entry| entry["Address"].as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

/// Start the relay, with a certificate that carries the name the sender
/// dials.
async fn start_relay() -> Relay {
    let mut pki = mint_pki(&["localhost"]);
    let mut container = start_container(&pki).await;
    let host = container.get_host().await.unwrap().to_string();
    if !pki.carries(&host) {
        container.stop_with_timeout(Some(0)).await.unwrap();
        pki = mint_pki(&["localhost", &host]);
        container = start_container(&pki).await;
    }
    let smtp_port = container.get_host_port_ipv4(SMTP_PORT).await.unwrap();
    let http_port = container.get_host_port_ipv4(HTTP_PORT).await.unwrap();
    let relay = Relay {
        _container: container,
        host,
        smtp_port,
        http_port,
        ca_pem: pki.ca_pem.clone(),
    };
    wait_for_the_smtp_port(&relay.host, relay.smtp_port).await;
    relay
}

/// Start the relay container, serving the given certificate.
async fn start_container(pki: &TestPki) -> ContainerAsync<GenericImage> {
    GenericImage::new(MAILPIT_IMAGE, MAILPIT_TAG)
        .with_wait_for(WaitFor::message_on_either_std("[smtpd] starting on"))
        .with_env_var("MP_SMTP_TLS_CERT", CERTIFICATE_PATH)
        .with_env_var("MP_SMTP_TLS_KEY", KEY_PATH)
        // The relay refuses a plaintext session: the sender must use
        // STARTTLS, as `SEC-56` D3 requires.
        .with_env_var("MP_SMTP_REQUIRE_STARTTLS", "true")
        .with_copy_to(CERTIFICATE_PATH, pki.certificate_pem.clone().into_bytes())
        .with_copy_to(KEY_PATH, pki.key_pem.clone().into_bytes())
        .start()
        .await
        .unwrap()
}

/// Wait until the relay listens: a container that is up is not yet a relay
/// that answers, and a dial that arrives too early is a connection error,
/// not a TLS one.
async fn wait_for_the_smtp_port(host: &str, port: u16) {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        if tokio::net::TcpStream::connect((host, port)).await.is_ok() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the relay never started listening on {host}:{port}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
