//! `GET /api/connectors/ingestible` shows a caller only its own tenant's
//! connectors (`SEC-16`). Before the fix the route returned every tenant's
//! hosts, usernames and secret names to anyone holding `ingest:read`.
//!
//! Seeded fixtures (`0002_seed_identity.sql`, `0042_tenant_provisioning.sql`):
//! Bayu (Data Engineer, holds `ingest:read`) is in Meridian Group, Andi (Data
//! Engineer) only in Meridian Retail, Fajar (Platform Admin, `*:*`) in all
//! three. The service identity is minted the way the orchestrator's is:
//! `ingest:read` scope, bearer token.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up,
};

const GROUP: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL: &str = "11111111-1111-4111-8111-000000000002";
const GROUP_CONNECTOR: &str = "conn-ingestible-group";
const RETAIL_CONNECTOR: &str = "conn-ingestible-retail";
const UNASSIGNED_CONNECTOR: &str = "conn-ingestible-unassigned";

/// A `sql`-adapter connector with an ingest spec, of `tenant` (`None`:
/// unassigned).
async fn seed_connector(app: &TestApp, id: &str, tenant: Option<&str>) {
    let tenant_id: Option<Uuid> = tenant.map(|t| t.parse().unwrap());
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, \
         name, type, direction, host, secret_ref, environment, tenant, \
         adapter, ingest_mode, dial) VALUES \
         ($1, $2, \
         $3, 'PostgreSQL', 'source', 'unused', 'env:CONNECTOR_PG_PASSWORD', \
         'production', 'meridian', 'sql', 'batch', \
         '{\"driver\":\"postgres\",\"host\":\"127.0.0.1\",\"port\":5432,\"database\":\"d\",\"user\":\"u\"}'::jsonb)",
    )
    .bind(id)
    .bind(tenant_id)
    // `connector.name` is unique (`connector_name_unique`), so each fixture
    // gets a name of its own, derived from its id (SEC-16 test fix).
    .bind(format!("ingestible test {id}"))
    .execute(&app.pool)
    .await
    .expect("seed a sql-adapter connector");
}

async fn seed_three(app: &TestApp) {
    seed_connector(app, GROUP_CONNECTOR, Some(GROUP)).await;
    seed_connector(app, RETAIL_CONNECTOR, Some(RETAIL)).await;
    seed_connector(app, UNASSIGNED_CONNECTOR, None).await;
}

/// The ids of the three fixtures that appear in `body`, and the status.
async fn ingestible(
    app: &TestApp,
    credential: (&str, &str),
    x_tenant: Option<&str>,
) -> (StatusCode, Vec<String>) {
    let mut request = Request::builder()
        .method("GET")
        .uri("/api/connectors/ingestible")
        .header(credential.0, credential.1);
    if let Some(tenant) = x_tenant {
        request = request.header("x-tenant", tenant);
    }
    let response = app
        .router
        .clone()
        .oneshot(request.body(Body::empty()).expect("build request"))
        .await
        .expect("router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let ids = body
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str())
                .filter(|id| [GROUP_CONNECTOR, RETAIL_CONNECTOR, UNASSIGNED_CONNECTOR].contains(id))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (status, ids)
}

/// The regression test for `SEC-16`: it fails before the fix, when a user of
/// Meridian Group with `ingest:read` also saw Meridian Retail's connector.
#[tokio::test]
async fn a_user_sees_their_own_tenants_connector_and_not_another_tenants() {
    let app = spin_up().await;
    seed_three(&app).await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    // Bayu is in Group and Logistics; naming Group keeps the test independent
    // of which membership comes first.
    let (status, ids) = ingestible(&app, ("cookie", &bayu), Some(GROUP)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids, vec![GROUP_CONNECTOR.to_owned()]);
}

/// A person in no tenant gets an empty list, not every tenant's rows.
#[tokio::test]
async fn a_user_in_no_tenant_gets_an_empty_list() {
    let app = spin_up().await;
    seed_three(&app).await;
    let user_id = create_principal_with_permissions(&app.pool, "ingest:read").await;
    let cookie = session_cookie_for_user(&app.pool, user_id).await;
    let (status, ids) = ingestible(&app, ("cookie", &cookie), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ids.is_empty(), "{ids:?}");
}

/// The orchestrator's service identity still reads every tenant's rows, and
/// the unassigned one: scheduled loads of every tenant depend on it.
#[tokio::test]
async fn the_service_identity_sees_every_connector() {
    const TOKEN: &str = "svc-ingestible-fixture-token";
    let app = spin_up().await;
    seed_three(&app).await;
    let identity = lakehouse_store::identity::create_service_identity(
        &app.pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-ingest-run".to_owned(),
            scopes: vec!["ingest:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .expect("seed the ingest service identity fixture");
    let identity_id: Uuid = identity.id.parse().expect("a UUID identity id");
    lakehouse_auth::service_token::ensure_service_credential(
        &app.pool,
        identity_id,
        &Secret::new(TOKEN.to_owned()),
    )
    .await
    .expect("seed the ingest service credential fixture");

    let bearer = format!("Bearer {TOKEN}");
    let (status, mut ids) = ingestible(&app, ("authorization", &bearer), None).await;
    assert_eq!(status, StatusCode::OK);
    ids.sort();
    assert_eq!(
        ids,
        vec![
            GROUP_CONNECTOR.to_owned(),
            RETAIL_CONNECTOR.to_owned(),
            UNASSIGNED_CONNECTOR.to_owned()
        ]
    );
}

/// An unrestricted administrator reads across tenants too.
#[tokio::test]
async fn an_unrestricted_administrator_sees_every_connector() {
    let app = spin_up().await;
    seed_three(&app).await;
    let fajar = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let (status, ids) = ingestible(&app, ("cookie", &fajar), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ids.len(), 3, "{ids:?}");
}

/// `X-Tenant` naming a tenant the caller is not in is 404, never a way to
/// read that tenant's rows.
#[tokio::test]
async fn an_x_tenant_the_caller_is_not_in_is_not_found() {
    let app = spin_up().await;
    seed_three(&app).await;
    let bayu = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    let (status, ids) = ingestible(&app, ("cookie", &bayu), Some(RETAIL)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(ids.is_empty());
}
