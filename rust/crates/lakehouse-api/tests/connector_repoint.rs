//! `SEC-14`: changing where a connector points never sends its stored
//! credential to the new place.
//!
//! Before the fix, `PUT /api/connectors/{id}/ingest-spec` replaced a
//! connector's `dial` and kept its stored `secret_ref`, so the next
//! `POST .../test`, discovery, delete or scheduled ingest resolved the old
//! credential and sent it to whatever host the `dial` now named. The same
//! reached a connector through `PUT .../credential` (a `dial` override that
//! kept the slot the request did not send) and through `PATCH` `host` for a
//! connector that dials from that column.
//!
//! # How these tests prove "nothing was dialled"
//!
//! The "new host" is a listener on loopback that these tests own. Loopback
//! is an internal address, which the `SEC-15` block refuses before any
//! dial — with the block on, a listener would stay untouched for the wrong
//! reason. So every fixture here that is meant to PROVE a non-dial turns
//! the block off (`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true`) and then
//! asserts that the listener received no connection. The one test that
//! leaves the block on says so, and asserts only the status.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::connectors::{CredentialKind, CredentialSource, derive_secret_ref};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

/// The fixed refusal, word for word (`REPOINT_NEEDS_CREDENTIALS` in
/// `routes/connectors.rs`).
const REPOINT_REFUSAL: &str = "This change points the connector at a different server, so every \
     credential it signs in with must be sent again in the same request. Nothing was saved and no \
     connection was made.";

/// A running app, the Data Engineer's session cookie, and the directory
/// managed credential files are written to (a temporary one: `/run/secrets`
/// does not exist outside Docker).
struct Fixture {
    app: TestApp,
    cookie: String,
    secrets: tempfile::TempDir,
}

async fn fixture(allow_internal: bool) -> Fixture {
    let secrets = tempfile::tempdir().unwrap();
    let mut env = HashMap::from([(
        "CONNECTOR_SECRETS_DIR".to_owned(),
        secrets.path().to_string_lossy().into_owned(),
    )]);
    if allow_internal {
        env.insert(
            "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
            "true".to_owned(),
        );
    }
    let app = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    Fixture {
        app,
        cookie,
        secrets,
    }
}

/// A loopback listener that accepts and immediately drops every connection
/// (so a probe that reaches it fails fast) and counts them.
struct Listener {
    port: u16,
    seen: Arc<AtomicUsize>,
}

impl Listener {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&seen);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                counter.fetch_add(1, Ordering::SeqCst);
                drop(stream);
            }
        });
        Self { port, seen }
    }

    /// How many connections arrived, after giving a late one time to land.
    async fn connections(&self) -> usize {
        tokio::time::sleep(Duration::from_millis(300)).await;
        self.seen.load(Ordering::SeqCst)
    }
}

async fn send(fx: &Fixture, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("cookie", &fx.cookie);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = fx
        .app
        .router
        .clone()
        .oneshot(builder.body(body).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn put_spec(fx: &Fixture, id: &str, body: Value) -> (StatusCode, Value) {
    send(
        fx,
        "PUT",
        &format!("/api/connectors/{id}/ingest-spec"),
        Some(body),
    )
    .await
}

fn sql_spec(host: &str, port: u16, database: &str, user: &str) -> Value {
    json!({
        "adapter": "sql",
        "ingestMode": "batch",
        "dial": {
            "driver": "postgres", "host": host, "port": port,
            "database": database, "user": user,
        },
        "sourceObjects": [],
    })
}

fn rest_spec(base_url: &str, auth: &str) -> Value {
    json!({
        "adapter": "rest",
        "ingestMode": "batch",
        "dial": {
            "baseUrl": base_url,
            "auth": { "type": auth },
            "pagination": { "type": "none" },
            "endpoints": [],
        },
        "sourceObjects": [],
    })
}

/// A `sql` connector of Meridian Group dialling `127.0.0.1:port`.
async fn seed_sql(pool: &PgPool, id: &str, port: u16) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, 'repoint sql', \
         'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', 'production', 'meridian', \
         'sql', 'batch', $3)",
    )
    .bind(id)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(json!({
        "driver": "postgres", "host": "127.0.0.1", "port": port,
        "database": "d", "user": "u",
    }))
    .execute(pool)
    .await
    .expect("seed a sql connector");
}

/// A `rest` connector of Meridian Group; with `basic` auth it has the two
/// credential slots (username, password).
async fn seed_rest(pool: &PgPool, id: &str, base_url: &str, basic: bool) {
    let secondary = basic.then_some("env:CONNECTOR_REST_REPOINT_PASSWORD");
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         secret_ref_secondary, environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, \
         'repoint rest', 'REST API', 'source', 'unused', 'env:CONNECTOR_REST_REPOINT_ACCESS_KEY', \
         $3, 'production', 'meridian', 'rest', 'batch', $4)",
    )
    .bind(id)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(secondary)
    .bind(rest_spec(base_url, if basic { "basic" } else { "bearer" })["dial"].clone())
    .execute(pool)
    .await
    .expect("seed a rest connector");
}

/// A connector from before `0033`: no adapter, no dial, and a `host` it
/// dials from.
async fn seed_legacy(pool: &PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant) VALUES ($1, $2, 'repoint legacy', 'PostgreSQL', 'source', \
         'lakehouse@legacy-db:5432/lakehouse', 'env:CONNECTOR_PG_PASSWORD', 'production', \
         'meridian')",
    )
    .bind(id)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .execute(pool)
    .await
    .expect("seed a legacy connector");
}

async fn dial_of(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar("SELECT dial FROM connector WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn refs_of(pool: &PgPool, id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT secret_ref, secret_ref_secondary FROM connector WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

fn stored_files(fx: &Fixture) -> usize {
    std::fs::read_dir(fx.secrets.path()).unwrap().count()
}

// ── The attack, and the refusal ──────────────────────────────────────────

/// The acceptance test of `SEC-14`. Re-pointing the seeded
/// `conn-pg-lakehouse` at a host the caller owns, without sending the
/// credential, is a 409 with the fixed text; the connector is unchanged; and
/// the connection test that followed the attack in the original report
/// connects to nobody.
#[tokio::test]
async fn re_pointing_the_seeded_connector_without_credentials_is_refused_and_never_dialled() {
    let fx = fixture(true).await;
    let attacker = Listener::start().await;
    let before = dial_of(&fx.app.pool, "conn-pg-lakehouse").await;
    let refs_before = refs_of(&fx.app.pool, "conn-pg-lakehouse").await;

    let (status, body) = put_spec(
        &fx,
        "conn-pg-lakehouse",
        sql_spec("127.0.0.1", attacker.port, "lakehouse", "lakehouse"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], REPOINT_REFUSAL);
    assert_eq!(dial_of(&fx.app.pool, "conn-pg-lakehouse").await, before);
    assert_eq!(
        refs_of(&fx.app.pool, "conn-pg-lakehouse").await,
        refs_before
    );

    // What the original attack did next: test the connector.
    let (status, _) = send(&fx, "POST", "/api/connectors/conn-pg-lakehouse/test", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        attacker.connections().await,
        0,
        "the listener standing in for the new host must receive nothing"
    );
}

/// The refusal comes before the save-time address check (`SEC-15`): with the
/// internal-address block on (the default), a re-point to a loopback host
/// without credentials is the 409, not the address check's 400, which would
/// have resolved the new host first. This test leaves the block on on
/// purpose; it asserts the status only.
#[tokio::test]
async fn the_refusal_comes_before_the_address_check() {
    let fx = fixture(false).await;
    let (status, body) = put_spec(
        &fx,
        "conn-pg-lakehouse",
        sql_spec("127.0.0.1", 5432, "lakehouse", "lakehouse"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], REPOINT_REFUSAL);
}

/// Host, port, database, driver and adapter each count as re-pointing a
/// `sql` connector; none writes anything.
#[tokio::test]
async fn a_change_of_host_port_database_driver_or_adapter_is_refused() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_sql(&fx.app.pool, "conn-sec14-sql", listener.port).await;
    let before = dial_of(&fx.app.pool, "conn-sec14-sql").await;
    let port = listener.port;

    let mut mysql = sql_spec("127.0.0.1", port, "d", "u");
    mysql["dial"]["driver"] = json!("mysql");
    let cdc = json!({
        "adapter": "cdc",
        "ingestMode": "cdc",
        "dial": {
            "driver": "postgres", "host": "127.0.0.1", "port": port, "database": "d",
            "user": "u", "slotName": "s", "publicationName": "p",
        },
        "sourceObjects": [],
    });
    for (what, spec) in [
        ("host", sql_spec("127.0.0.2", port, "d", "u")),
        (
            "port",
            sql_spec("127.0.0.1", port.wrapping_add(1), "d", "u"),
        ),
        ("database", sql_spec("127.0.0.1", port, "other", "u")),
        ("driver", mysql),
        ("adapter", cdc),
    ] {
        let (status, body) = put_spec(&fx, "conn-sec14-sql", spec).await;
        assert_eq!(status, StatusCode::CONFLICT, "{what}: {body}");
        assert_eq!(body["error"], REPOINT_REFUSAL, "{what}");
        assert_eq!(
            dial_of(&fx.app.pool, "conn-sec14-sql").await,
            before,
            "{what}"
        );
    }
    assert_eq!(listener.connections().await, 0);
}

/// A `rest` connector is re-pointed by the scheme, host or port of its
/// `baseUrl`; the default port is not a change in either direction.
#[tokio::test]
async fn a_change_of_rest_base_url_is_refused_and_a_default_port_is_not_a_change() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_rest(&fx.app.pool, "conn-sec14-rest", "http://127.0.0.1", false).await;
    let before = dial_of(&fx.app.pool, "conn-sec14-rest").await;

    for (what, base_url) in [
        ("host", "http://127.0.0.2".to_owned()),
        ("port", format!("http://127.0.0.1:{}", listener.port)),
        ("scheme", "https://127.0.0.1".to_owned()),
    ] {
        let (status, body) = put_spec(&fx, "conn-sec14-rest", rest_spec(&base_url, "bearer")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{what}: {body}");
        assert_eq!(
            dial_of(&fx.app.pool, "conn-sec14-rest").await,
            before,
            "{what}"
        );
    }

    // Writing the scheme's default port explicitly (and a path) is the same
    // target: saved, no credentials needed.
    let (status, body) = put_spec(
        &fx,
        "conn-sec14-rest",
        rest_spec("http://127.0.0.1:80/v2", "bearer"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        dial_of(&fx.app.pool, "conn-sec14-rest").await["baseUrl"],
        "http://127.0.0.1:80/v2"
    );
    assert_eq!(listener.connections().await, 0);
}

/// A save that keeps the target (another user, other objects, a schedule)
/// needs no credentials, as before.
#[tokio::test]
async fn a_save_that_keeps_the_target_needs_no_credentials() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_sql(&fx.app.pool, "conn-sec14-same", listener.port).await;

    let mut spec = sql_spec("127.0.0.1", listener.port, "d", "someone_else");
    spec["sourceObjects"] = json!([{ "name": "public.orders", "target": "orders" }]);
    spec["scheduleCron"] = json!("0 * * * *");
    let (status, body) = put_spec(&fx, "conn-sec14-same", spec).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        dial_of(&fx.app.pool, "conn-sec14-same").await["user"],
        "someone_else"
    );
}

/// The first-configuration boundary: a connector that has no adapter and an
/// empty dial takes its first dial without credentials (the console's create
/// flow); the next change of host is a re-point.
#[tokio::test]
async fn the_first_dial_of_a_connector_is_saved_and_the_next_host_is_refused() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_legacy(&fx.app.pool, "conn-sec14-first").await;

    let (status, body) = put_spec(
        &fx,
        "conn-sec14-first",
        sql_spec("127.0.0.1", listener.port, "d", "u"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, body) = put_spec(
        &fx,
        "conn-sec14-first",
        sql_spec("127.0.0.2", listener.port, "d", "u"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
}

// ── With credentials ─────────────────────────────────────────────────────

/// With the credential in the same request, the new settings are probed with
/// that credential and, when the probe passes, the spec and the credential
/// are saved together. The new target here is the test Postgres server
/// itself, so the probe really connects.
#[tokio::test]
async fn a_re_point_with_credentials_is_probed_and_saved_together() {
    let url = lakehouse_test_support::database_url();
    let (user, password, host, port) = parse_postgres_url(&url);
    let fx = fixture(true).await;
    let (database,): (String,) = sqlx::query_as("SELECT current_database()")
        .fetch_one(&fx.app.pool)
        .await
        .unwrap();

    let mut spec = sql_spec(&host, port, &database, &user);
    spec["credential"] = json!({ "primary": { "kind": "password", "value": password } });
    let (status, body) = put_spec(&fx, "conn-pg-lakehouse", spec).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["dial"]["host"], host.as_str());

    // The spec and the new ref landed together; the value is in the store,
    // not in the database.
    let managed = derive_secret_ref(
        "conn-pg-lakehouse",
        CredentialSource::Managed,
        CredentialKind::Password,
    );
    assert_eq!(refs_of(&fx.app.pool, "conn-pg-lakehouse").await.0, managed);
    assert_eq!(
        dial_of(&fx.app.pool, "conn-pg-lakehouse").await["host"],
        host.as_str()
    );
    let stored = std::fs::read_to_string(
        fx.secrets
            .path()
            .join("connector_managed_conn_pg_lakehouse_password"),
    )
    .unwrap();
    assert_eq!(stored, password);

    // Audited as a re-point: the slots and whether it was verified, no ref,
    // no value.
    let args: Value = sqlx::query_scalar(
        "SELECT args FROM audit_event WHERE action = 'connector.repoint' \
         AND resource_id = 'conn-pg-lakehouse'",
    )
    .fetch_one(&fx.app.pool)
    .await
    .expect("one connector.repoint audit event");
    assert_eq!(args, json!({ "slots": ["primary"], "verified": true }));
    assert!(!args.to_string().contains(&password));
}

/// A probe the source rejects (here: the "new host" hangs up) is a 422 and
/// saves nothing: not the spec, not the ref, not the file. The probe did
/// connect — to the new host, with the credential of this request only.
#[tokio::test]
async fn a_re_point_whose_probe_fails_saves_nothing() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    let before = dial_of(&fx.app.pool, "conn-pg-lakehouse").await;
    let refs_before = refs_of(&fx.app.pool, "conn-pg-lakehouse").await;

    let mut spec = sql_spec("127.0.0.1", listener.port, "lakehouse", "lakehouse");
    spec["credential"] = json!({ "primary": { "kind": "password", "value": "new-password" } });
    let (status, body) = put_spec(&fx, "conn-pg-lakehouse", spec).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or_default()
            .contains("NOT saved"),
        "{body}"
    );

    assert!(listener.connections().await >= 1, "the probe must have run");
    assert_eq!(dial_of(&fx.app.pool, "conn-pg-lakehouse").await, before);
    assert_eq!(
        refs_of(&fx.app.pool, "conn-pg-lakehouse").await,
        refs_before
    );
    assert_eq!(
        stored_files(&fx),
        0,
        "no credential file may be left behind"
    );
}

/// A connector that reads two credentials (REST basic auth: username and
/// password) needs both. One of two is a 409 and the new host is never
/// contacted — the stored second credential must not travel there.
#[tokio::test]
async fn one_credential_slot_of_two_is_refused_and_never_dialled() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_rest(&fx.app.pool, "conn-sec14-basic", "http://127.0.0.1", true).await;
    let before = dial_of(&fx.app.pool, "conn-sec14-basic").await;
    let refs_before = refs_of(&fx.app.pool, "conn-sec14-basic").await;

    let mut spec = rest_spec(&format!("http://127.0.0.1:{}", listener.port), "basic");
    spec["credential"] = json!({ "primary": { "kind": "access_key", "value": "someone" } });
    let (status, body) = put_spec(&fx, "conn-sec14-basic", spec.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], REPOINT_REFUSAL);
    assert_eq!(listener.connections().await, 0);
    assert_eq!(dial_of(&fx.app.pool, "conn-sec14-basic").await, before);
    assert_eq!(refs_of(&fx.app.pool, "conn-sec14-basic").await, refs_before);

    // Both slots: now the probe runs, with those two values.
    spec["credential"]["secondary"] = json!({ "kind": "password", "value": "a-password" });
    let (status, _) = put_spec(&fx, "conn-sec14-basic", spec).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(listener.connections().await >= 1);
    assert_eq!(dial_of(&fx.app.pool, "conn-sec14-basic").await, before);
}

// ── PUT .../credential with a dial ───────────────────────────────────────

/// The `dial` override of `PUT .../credential` was the same hole: sending one
/// slot plus a new `dial` probed the new host with the stored other slot.
/// Now a `dial` that changes the target needs every slot, otherwise 409
/// before any probe.
#[tokio::test]
async fn a_credential_request_with_a_re_pointing_dial_needs_every_slot() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_rest(&fx.app.pool, "conn-sec14-cred", "http://127.0.0.1", true).await;
    let dial = rest_spec(&format!("http://127.0.0.1:{}", listener.port), "basic")["dial"].clone();

    let (status, body) = send(
        &fx,
        "PUT",
        "/api/connectors/conn-sec14-cred/credential",
        Some(json!({
            "primary": { "kind": "access_key", "value": "someone" },
            "dial": dial,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], REPOINT_REFUSAL);
    assert_eq!(
        listener.connections().await,
        0,
        "the stored second credential must not be sent to the new host"
    );

    // Both slots: probed (and refused here, the listener hangs up).
    let (status, _) = send(
        &fx,
        "PUT",
        "/api/connectors/conn-sec14-cred/credential",
        Some(json!({
            "primary": { "kind": "access_key", "value": "someone" },
            "secondary": { "kind": "password", "value": "a-password" },
            "dial": dial,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(listener.connections().await >= 1);
    assert_eq!(stored_files(&fx), 0);
}

// ── PATCH host ───────────────────────────────────────────────────────────

/// `PATCH` `host` on a connector that dials from that column (one with no
/// adapter) is refused; the same value is accepted; and for a connector with
/// an adapter, where the column is a label, it still works.
#[tokio::test]
async fn patching_the_host_of_a_connector_that_dials_from_it_is_refused() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_legacy(&fx.app.pool, "conn-sec14-legacy").await;
    seed_sql(&fx.app.pool, "conn-sec14-label", listener.port).await;

    let (status, body) = send(
        &fx,
        "PATCH",
        "/api/connectors/conn-sec14-legacy",
        Some(json!({ "host": "lakehouse@attacker:5432/lakehouse" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let host: String = sqlx::query_scalar("SELECT host FROM connector WHERE id = $1")
        .bind("conn-sec14-legacy")
        .fetch_one(&fx.app.pool)
        .await
        .unwrap();
    assert_eq!(host, "lakehouse@legacy-db:5432/lakehouse");

    // The value it already has changes nothing.
    let (status, body) = send(
        &fx,
        "PATCH",
        "/api/connectors/conn-sec14-legacy",
        Some(json!({ "host": "Lakehouse@legacy-db:5432/lakehouse" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A connector with an adapter dials from `dial`: its `host` is a label.
    let (status, body) = send(
        &fx,
        "PATCH",
        "/api/connectors/conn-sec14-label",
        Some(json!({ "host": "a new label" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

// ── The assistant ────────────────────────────────────────────────────────

/// The assistant's `set_ingest_spec` tool never carries a credential, so a
/// re-point through it ends in the route's fixed 409, handed to the model as
/// its `error`; the connector is unchanged.
#[tokio::test]
async fn the_assistants_tool_cannot_re_point_a_connector() {
    let fx = fixture(true).await;
    let listener = Listener::start().await;
    seed_sql(&fx.app.pool, "conn-sec14-tool", listener.port).await;
    let before = dial_of(&fx.app.pool, "conn-sec14-tool").await;

    let (status, body) = send(
        &fx,
        "POST",
        "/api/ai/tool",
        Some(json!({
            "tool": "set_ingest_spec",
            "mode": "build",
            "args": {
                "id": "conn-sec14-tool",
                "adapter": "sql",
                "ingestMode": "batch",
                "dial": {
                    "driver": "postgres", "host": "127.0.0.2", "port": listener.port,
                    "database": "d", "user": "u",
                },
                "sourceObjects": [],
                // A model that invents a credential still cannot send it:
                // only the five spec keys are forwarded.
                "credential": { "primary": { "kind": "password", "value": "invented" } },
                "confirmed": true,
            },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"]["error"], REPOINT_REFUSAL, "{body}");
    assert_eq!(dial_of(&fx.app.pool, "conn-sec14-tool").await, before);
    assert_eq!(listener.connections().await, 0);
}

fn parse_postgres_url(url: &str) -> (String, String, String, u16) {
    let rest = url
        .strip_prefix("postgres://")
        .expect("lakehouse_test_support::database_url is a postgres:// URL");
    let (auth, host_port_db) = rest.split_once('@').expect("user:pass@host:port/db");
    let (user, password) = auth.split_once(':').unwrap_or((auth, ""));
    let (host_port, _db) = host_port_db.split_once('/').unwrap_or((host_port_db, ""));
    let (host, port) = host_port.rsplit_once(':').expect("host:port");
    (
        user.to_owned(),
        password.to_owned(),
        host.to_owned(),
        port.parse().expect("numeric port"),
    )
}
