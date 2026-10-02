//! `PUT /api/connectors/{id}/ingest-spec` refuses a `sourceObjects[].target`
//! that is a raw table an uploaded file was loaded into (T8 of
//! `docs/superpowers/plans/2026-10-02-upload-file.md`, ADR 0014, decision 5).
//!
//! An upload may never load into a connector's table; this is the other half,
//! so a scheduled connector cannot replace or append to what a person
//! uploaded. The check sits in the handler, not in a layer, because the
//! copilot's `set_ingest_spec` tool calls the handler without the router.
//!
//! `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true` lets the saved dial name a
//! loopback address: the save-time SSRF check resolves it, and the point here
//! is what happens to the targets, not the dial.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::uploads::{self, LoadMode, NewUpload};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const GROUP: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";

async fn app() -> TestApp {
    spin_up_with_env(&HashMap::from([(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    )]))
    .await
}

/// A `sql` connector of Meridian Group, which Bayu (Data Engineer) manages.
async fn seed_connector(app: &TestApp, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, \
         name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, '11111111-1111-4111-8111-000000000001', \
         'upload table test', 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"127.0.0.1\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .bind(id)
    .execute(&app.pool)
    .await
    .expect("seed a sql-adapter connector");
}

/// An upload of `tenant` that went the way `status` says, into `table`.
/// `ingested` is a load that succeeded; the others are a claim that has not
/// (or not yet) loaded anything.
async fn seed_upload(app: &TestApp, tenant: &str, id: &str, table: &str, status: &str) {
    let tenant_id: Uuid = tenant.parse().unwrap();
    uploads::insert(
        &app.pool,
        &NewUpload {
            id,
            original_filename: "stock.csv",
            storage_key: &format!("uploads/{tenant}/{id}.csv"),
            content_type: "text/csv",
            size_bytes: 8,
            sha256: "",
            uploaded_by: "Seeded Uploader",
            tenant_id,
        },
    )
    .await
    .unwrap();
    let options = json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 });
    uploads::mark_ingesting(
        &app.pool,
        id,
        &options,
        table,
        LoadMode::Replace,
        Some("run-1"),
    )
    .await
    .unwrap()
    .unwrap();
    match status {
        "ingested" => {
            uploads::mark_finished(&app.pool, id, Some("run-1"), None, Some(1))
                .await
                .unwrap()
                .unwrap();
        }
        "failed" => {
            uploads::mark_finished(
                &app.pool,
                id,
                Some("run-1"),
                Some("The load into the table failed."),
                None,
            )
            .await
            .unwrap()
            .unwrap();
        }
        "ingesting" => {}
        other => panic!("unknown status {other}"),
    }
}

fn spec(targets: &[&str]) -> Value {
    let objects: Vec<Value> = targets
        .iter()
        .map(|target| json!({ "name": format!("public.{target}"), "target": target }))
        .collect();
    json!({
        "adapter": "sql",
        "ingestMode": "batch",
        "dial": { "driver": "postgres", "host": "127.0.0.1", "port": 5432, "database": "d", "user": "u" },
        "sourceObjects": objects,
    })
}

async fn put_spec(app: &TestApp, id: &str, body: &Value) -> (StatusCode, Value) {
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/connectors/{id}/ingest-spec"))
                .header("cookie", cookie)
                .header("x-tenant", GROUP)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn stored_targets(app: &TestApp, id: &str) -> Value {
    sqlx::query_scalar("SELECT source_objects FROM connector WHERE id = $1")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// A table an upload loaded is refused, whichever object names it, whichever
/// tenant's upload it was, and whether or not the upload still exists; the
/// connector is left as it was.
#[tokio::test]
async fn a_connector_may_not_take_a_table_an_upload_loaded() {
    let app = app().await;
    seed_connector(&app, "conn-upload-table").await;
    seed_upload(&app, GROUP, "up-group", "orders_raw", "ingested").await;
    seed_upload(&app, RETAIL, "up-retail", "stock_raw", "ingested").await;
    assert!(uploads::soft_delete(&app.pool, "up-retail").await.unwrap());
    let before = stored_targets(&app, "conn-upload-table").await;

    for (targets, refused) in [
        (vec!["orders_raw"], "orders_raw"),
        (vec!["customers_raw", "orders_raw"], "orders_raw"),
        // Another tenant's upload, since deleted: raw table names are shared,
        // and deleting an upload keeps its table.
        (vec!["stock_raw"], "stock_raw"),
    ] {
        let (status, body) = put_spec(&app, "conn-upload-table", &spec(&targets)).await;

        assert_eq!(status, StatusCode::CONFLICT, "{targets:?}: {body}");
        let message = body["error"].as_str().unwrap();
        assert!(message.contains(refused), "{message}");
        assert!(message.contains("uploaded file"), "{message}");
        assert_eq!(
            stored_targets(&app, "conn-upload-table").await,
            before,
            "nothing was saved"
        );
    }
}

/// A target no upload loaded saves as it always did.
#[tokio::test]
async fn an_ordinary_target_still_saves() {
    let app = app().await;
    seed_connector(&app, "conn-ordinary").await;
    seed_upload(&app, GROUP, "up-group", "orders_raw", "ingested").await;

    let (status, body) = put_spec(
        &app,
        "conn-ordinary",
        &spec(&["customers_raw", "items_raw"]),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["sourceObjects"].as_array().unwrap().len(), 2);
    let stored = stored_targets(&app, "conn-ordinary").await;
    assert_eq!(stored[0]["target"], "customers_raw");
    assert_eq!(stored[1]["target"], "items_raw");

    // And a spec with no object to name a target for.
    let (status, body) = put_spec(&app, "conn-ordinary", &spec(&[])).await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

/// What the rule does not cover, pinned so a change is a decision: a table an
/// upload has only claimed (its load is running, or failed) is not one an
/// upload loaded, so a connector may still take it.
#[tokio::test]
async fn a_table_an_upload_only_claimed_does_not_block_a_connector() {
    let app = app().await;
    seed_connector(&app, "conn-claimed").await;
    seed_upload(&app, GROUP, "up-running", "running_raw", "ingesting").await;
    seed_upload(&app, GROUP, "up-failed", "failed_raw", "failed").await;

    let (status, body) =
        put_spec(&app, "conn-claimed", &spec(&["running_raw", "failed_raw"])).await;

    assert_eq!(status, StatusCode::OK, "{body}");
}

/// The rest of the spec's validation answers as it did: a load mode the job
/// cannot run and a dial that does not parse are still 400.
#[tokio::test]
async fn the_other_validation_of_the_spec_is_unchanged() {
    let app = app().await;
    seed_connector(&app, "conn-validation").await;

    let mut bad_mode = spec(&["orders_raw"]);
    bad_mode["sourceObjects"][0]["loadMode"] = json!("merge");
    let (status, body) = put_spec(&app, "conn-validation", &bad_mode).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    let mut bad_dial = spec(&["orders_raw"]);
    bad_dial["dial"] = json!({});
    let (status, _) = put_spec(&app, "conn-validation", &bad_dial).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
