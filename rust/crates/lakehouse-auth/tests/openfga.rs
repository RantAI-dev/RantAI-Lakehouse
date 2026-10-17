//! Tests for `lakehouse_auth::openfga::LakekeeperAdminClient` against a
//! `wiremock` server standing in for `Lakekeeper`'s management API —
//! never a real `Lakekeeper` instance. See `openfga.rs`'s module doc
//! comment for why this file (and the module it tests) is named
//! `openfga` while never speaking `OpenFGA`'s own protocol.
//!
//! No `#[sqlx::test]`/Postgres here: this client makes no database call,
//! so unlike `tests/oidc.rs`'s two-tier split these tests are all plain
//! `#[tokio::test]`s.
//!
//! Each test asserts the exact request shape (method, path, and — where
//! it matters to the behavior under test — body) `Lakekeeper` actually
//! receives, not merely that the call returned `Ok` — a mock that only
//! checks the return value would not catch a wrong grant payload reaching
//! `Lakekeeper`. The "no POST mock registered" tests additionally
//! rely on `wiremock`'s own unmatched-request panic on `MockServer` drop
//! to prove the client never attempts a call it should have skipped.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_auth::Secret;
use lakehouse_auth::openfga::{LakekeeperAdminClient, OpenfgaError, TenantWarehouseStorage};
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A fixed [`TenantWarehouseStorage`] fixture every `ensure_warehouse`
/// test below shares, so each test's `body_json` assertion is the only
/// thing that varies, not the storage settings themselves.
fn test_storage() -> TenantWarehouseStorage {
    TenantWarehouseStorage {
        endpoint: "http://rustfs.test:9000".to_owned(),
        region: "us-east-1".to_owned(),
        bucket: "lakehouse-warehouse".to_owned(),
        path_style_access: true,
        sts_enabled: true,
        sts_role_arn: "arn:aws:iam::000000000000:role/lakekeeper".to_owned(),
        access_key: "tenant-warehouse-access-key".to_owned(),
        secret_key: Secret::new("tenant-warehouse-secret-key"),
    }
}

#[tokio::test]
async fn ensure_warehouse_reuses_an_existing_warehouse_by_name_instead_of_recreating() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "warehouses": [{"id": "wh-existing", "name": "tenant-acme"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    // No POST mock registered at all — if the client tried to create,
    // wiremock's unmatched-request panic on drop catches it.
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    let id = client
        .ensure_warehouse("tenant-acme", "lakehouse-warehouse/acme/", &test_storage())
        .await
        .unwrap();
    assert_eq!(id, "wh-existing");
}

/// The full body `POST /management/v1/warehouse` must carry — every field
/// `Lakekeeper`'s `S3Profile`/`S3Credential` require, matching
/// `docker-compose.yml`'s `lakekeeper-warehouse-init` body field-for-field
/// (see `TenantWarehouseStorage`'s doc comment). The prior body
/// (`{"warehouse-name", "storage-profile": {"prefix"}}`) is exactly what
/// made every real tenant-provisioning call 422 — this test pins the fix,
/// not just that `ensure_warehouse` returns `Ok`.
#[tokio::test]
async fn ensure_warehouse_creates_when_no_warehouse_with_that_name_exists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"warehouses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse"))
        .and(body_json(json!({
            "warehouse-name": "tenant-acme",
            "storage-profile": {
                "type": "s3",
                "bucket": "lakehouse-warehouse",
                "region": "us-east-1",
                "endpoint": "http://rustfs.test:9000",
                "path-style-access": true,
                "sts-enabled": true,
                "sts-role-arn": "arn:aws:iam::000000000000:role/lakekeeper",
                "key-prefix": "lakehouse-warehouse/acme/"
            },
            "storage-credential": {
                "type": "s3",
                "credential-type": "access-key",
                "aws-access-key-id": "tenant-warehouse-access-key",
                "aws-secret-access-key": "tenant-warehouse-secret-key"
            }
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"warehouse-id": "wh-new"})))
        .expect(1)
        .mount(&server)
        .await;
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    let id = client
        .ensure_warehouse("tenant-acme", "lakehouse-warehouse/acme/", &test_storage())
        .await
        .unwrap();
    assert_eq!(id, "wh-new");
}

/// A `4xx` from `Lakekeeper` (our request rejected — e.g. a malformed
/// `storage-profile`) is classified as [`OpenfgaError::Rejected`], not
/// [`OpenfgaError::Server`] — the distinction `lakehouse_api::error::
/// provisioning_unavailable` uses to give an honest message category
/// without ever repeating Lakekeeper's own response text.
#[tokio::test]
async fn ensure_warehouse_classifies_a_4xx_as_rejected_not_server() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"warehouses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(422).set_body_json(json!({
            "error": {"message": "storage-profile: missing field 'type'"}
        })))
        .mount(&server)
        .await;
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    let err = client
        .ensure_warehouse("tenant-acme", "lakehouse-warehouse/acme/", &test_storage())
        .await
        .unwrap_err();
    assert!(matches!(err, OpenfgaError::Rejected));
}

/// A `5xx` from `Lakekeeper` is classified as [`OpenfgaError::Server`],
/// distinct from a `4xx` — see the `_rejected_not_server` test above.
#[tokio::test]
async fn ensure_warehouse_classifies_a_5xx_as_server() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"warehouses": []})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/management/v1/warehouse"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    let err = client
        .ensure_warehouse("tenant-acme", "lakehouse-warehouse/acme/", &test_storage())
        .await
        .unwrap_err();
    assert!(matches!(err, OpenfgaError::Server));
}

#[tokio::test]
async fn grant_machine_principals_sends_the_five_fixed_grants_lakekeeper_authz_init_sends() {
    let server = MockServer::start().await;
    // Matches `docker-compose.yml`'s `lakekeeper-authz-init` `grant()`
    // calls exactly (principal, verb set, and the `oidc~<principal>`
    // user shape) — asserted here rather than only "returned Ok" so a
    // regression that silently dropped or reordered a grant fails this
    // test, not just a production tenant-provisioning run.
    let expected_grants: &[(&str, &[&str])] = &[
        ("rust-iceberg", &["create", "modify", "select"]),
        ("debezium", &["create", "modify", "select"]),
        ("dlt", &["create", "modify", "select"]),
        ("clickhouse-reader", &["select", "modify"]),
        ("trino", &["select", "modify"]),
    ];
    for (principal, verbs) in expected_grants {
        let writes: Vec<_> = verbs
            .iter()
            .map(|verb| json!({ "type": verb, "user": format!("oidc~{principal}") }))
            .collect();
        Mock::given(method("POST"))
            .and(path(
                "/management/v1/permissions/warehouse/wh-1/assignments",
            ))
            .and(body_json(json!({ "writes": writes })))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
    }
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    client.grant_machine_principals("wh-1").await.unwrap();
}

#[tokio::test]
async fn grant_machine_principals_treats_409_as_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/permissions/warehouse/wh-1/assignments",
        ))
        .respond_with(ResponseTemplate::new(409))
        .mount(&server)
        .await;
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    client.grant_machine_principals("wh-1").await.unwrap();
}

#[tokio::test]
async fn grant_machine_principals_fails_on_an_unexpected_status() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/management/v1/permissions/warehouse/wh-1/assignments",
        ))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let client = LakekeeperAdminClient::new(server.uri(), Secret::new("admin-token"));
    assert!(client.grant_machine_principals("wh-1").await.is_err());
}
