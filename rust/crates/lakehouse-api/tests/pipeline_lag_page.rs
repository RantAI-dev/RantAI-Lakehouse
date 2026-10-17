//! BLD-2 review R4 BLOCKER 1 (a) / decision R16: the pipeline PAGES must
//! answer "is this pipeline blocked by an exhausted upstream?" from the
//! SAME chain walk the check uses, never from `dependsOn` alone.
//!
//! The failure being proven is one a unit test of `upstream_stop_of`
//! cannot show: P lists no `dependsOn` entry because its console record
//! never added one, yet P's source is U's target — the exact edge
//! (`target = ANY($2)` in `upstream_downstream_pipelines_conn`) the check
//! reads U on. Before this fix the check dutifully scheduled U while both
//! pages showed P as merely unmeasured. Both surfaces are asserted here:
//! the list row's `lag` and the detail page's `lag` must both name U once
//! U's retries are exhausted.
//!
//! `ClickHouse`, `Dagster` and Lakekeeper stay dead (`127.0.0.1:1`), as
//! everywhere in this directory: the Dagster-job half of the list is
//! deliberately refused in this two-tenant database, so no dead upstream
//! is ever reached and the responses are pure 200s over the authored half.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use time::OffsetDateTime;
use tower::ServiceExt;
use uuid::Uuid;

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_store::pipelines::{self, CreatePipelineInput, TargetLag};
use sqlx::PgPool;

/// The `CATALOG_TENANT_ID` a member of `spin_up_with_member`'s tenant never owns:
/// a fixed UUID (there is no such tenant row, which is fine — the config
/// key only needs to parse), so the list route refuses the Dagster-job
/// half instead of asking the dead `Dagster` upstream.
fn a_qualifying_but_unowned_catalog_tenant() -> Uuid {
    Uuid::from_u128(1)
}

async fn provision_tenant(pool: &PgPool, prefix: &str) -> Uuid {
    let tenant = lakehouse_store::identity::create_tenant(
        pool,
        &lakehouse_store::identity::CreateTenantInput {
            name: prefix.to_owned(),
            slug: format!("{prefix}-{}", uuid::Uuid::new_v4().simple()),
            plan: "starter".to_owned(),
            residency: "in-region".to_owned(),
        },
    )
    .await
    .expect("create a tenant");
    tenant.id.parse().expect("a UUID tenant id")
}

async fn set_ready(pool: &PgPool, id: &str) {
    pipelines::set_status(pool, id, "ready")
        .await
        .expect("mark the pipeline ready")
        .expect("the row comes back");
}

async fn get_json(app: &TestApp, path: &str, cookie: &str) -> (StatusCode, Value) {
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 128 * 1024)
        .await
        .expect("read body");
    let body =
        serde_json::from_slice(&bytes).expect("both asserted routes answer with a JSON body");
    (status, body)
}

/// A `pipeline:read` member of one tenant (the one whose pipelines are
/// seeded), with a second tenant and a `CATALOG_TENANT_ID` it does not
/// own so the Dagster-job half of the list is refused without a round
/// trip to the dead upstream.
async fn spin_up_with_member(pool_user: &str) -> (TestApp, Uuid, String) {
    let mut overrides = std::collections::HashMap::new();
    overrides.insert(
        "CATALOG_TENANT_ID".to_owned(),
        a_qualifying_but_unowned_catalog_tenant().to_string(),
    );
    let app = spin_up_with_env(&overrides).await;
    let tenant = provision_tenant(&app.pool, pool_user).await;
    // The refusal rule counts the deployment's tenants: there must be
    // more than one, or a member is never refused.
    provision_tenant(&app.pool, "lag-page-else").await;

    let user = create_principal_with_permissions(&app.pool, "pipeline:read").await;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user)
        .bind(tenant)
        .execute(&app.pool)
        .await
        .expect("attach the member to the tenant");
    let cookie = session_cookie_for_user(&app.pool, user).await;
    (app, tenant, cookie)
}

async fn create_pipeline(
    pool: &PgPool,
    name: &str,
    tenant: Uuid,
    target_lag: Option<TargetLag>,
    source: &str,
    target: &str,
    depends_on: Vec<String>,
    max_retries: Option<i16>,
) -> String {
    let created = pipelines::create_pipeline(
        pool,
        &CreatePipelineInput {
            name: name.to_owned(),
            kind: "batch".to_owned(),
            source_zone: source
                .split_once('.')
                .expect("a zone.table source")
                .0
                .to_owned(),
            source_table: source
                .split_once('.')
                .expect("a zone.table source")
                .1
                .to_owned(),
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            target_zone: target
                .split_once('.')
                .expect("a zone.table target")
                .0
                .to_owned(),
            target_table: target
                .split_once('.')
                .expect("a zone.table target")
                .1
                .to_owned(),
            schedule: "manual".to_owned(),
            owner: None,
            description: None,
            max_retries,
            tenant_id: Some(tenant),
            depends_on,
            connector_id: None,
            id: None,
            target_lag,
            run_as: None,
        },
        None,
    )
    .await
    .expect("seed the pipeline");
    created.id
}

/// An eighteen-hour-old check that has NOT consumed what it was asked:
/// the shape an exhausted U's state row has (requested tokens set,
/// consumed `None`, attempt counter past `max_retries`).
async fn seed_exhausted_state(pool: &PgPool, id: &str, tenant: Uuid, attempt: i32) {
    let now = OffsetDateTime::now_utc();
    pipelines::upsert_lag_state(
        pool,
        &pipelines::LagStateUpdate {
            pipeline_id: id.to_owned(),
            tenant_id: Some(tenant),
            consumed_tokens: None,
            consumed_at: None,
            pending_since: Some(now - time::Duration::hours(18)),
            requested_run_key: Some(format!("pl-run-key:{id}:0")),
            requested_tokens: Some(serde_json::json!({"bronze.raw": "fp:1"})),
            requested_at: Some(now),
            request_attempt: attempt,
            last_check_at: now,
            last_status: "not_measured".to_owned(),
            behind_streak: 0,
        },
    )
    .await
    .expect("seed the lag state");
}

/// A checked P that has never consumed anything and is not itself
/// exhausted — so `lag_json` would answer "not measured" for it and the
/// ONLY way the pages can track U is through the chain walk.
async fn seed_unmeasured_state(pool: &PgPool, id: &str, tenant: Uuid) {
    let now = OffsetDateTime::now_utc();
    pipelines::upsert_lag_state(
        pool,
        &pipelines::LagStateUpdate {
            pipeline_id: id.to_owned(),
            tenant_id: Some(tenant),
            consumed_tokens: None,
            consumed_at: None,
            pending_since: Some(now - time::Duration::hours(1)),
            requested_run_key: None,
            requested_tokens: None,
            requested_at: None,
            request_attempt: 0,
            last_check_at: now,
            last_status: "not_measured".to_owned(),
            behind_streak: 0,
        },
    )
    .await
    .expect("seed the lag state");
}

/// P's source is U's target, P's `dependsOn` is empty: the exact shape a
/// chain-scheduled read has and a `dependsOn`-mirrored read does not.
#[tokio::test]
async fn a_lag_pipeline_reading_its_upstream_only_through_the_source_table_names_that_upstream_on_both_pages()
 {
    let (app, tenant, cookie) = spin_up_with_member("lag-page-owned").await;

    let u_id = create_pipeline(
        &app.pool,
        "lag-page-upstream",
        tenant,
        Some(TargetLag::Downstream),
        "bronze.raw",
        "silver.orders_upstream",
        Vec::new(),
        Some(0),
    )
    .await;
    set_ready(&app.pool, &u_id).await;
    // attempt 1 > max_retries 0, asked and never consumed: exhausted
    // under `upstream_retries_exhausted`.
    seed_exhausted_state(&app.pool, &u_id, tenant, 1).await;

    let p_id = create_pipeline(
        &app.pool,
        "lag-page-downstream",
        tenant,
        Some(TargetLag::Lag { minutes: 10 }),
        "silver.orders_upstream",
        "gold.p_orders",
        Vec::new(),
        None,
    )
    .await;
    set_ready(&app.pool, &p_id).await;
    seed_unmeasured_state(&app.pool, &p_id, tenant).await;

    let (list_status, list_body) = get_json(&app, "/api/pipelines", &cookie).await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(
        list_body["dagsterJobs"]["supported"], false,
        "the dead Dagster upstream must be refused, never consulted"
    );
    let row = list_body["pipelines"]
        .as_array()
        .expect("a pipelines array")
        .iter()
        .find(|row| row["id"].as_str() == Some(p_id.as_str()))
        .expect("the lag pipeline is listed");
    assert_eq!(
        row["lag"]["status"], "behind",
        "a P blocked by an exhausted U is behind, not unmeasured"
    );
    // BLD-2 review R4 BLOCKER 1 (b): the exhausted-Upstream reason is its
    // own sentence; U was seeded with max_retries 0, so one run failed.
    assert_eq!(
        row["lag"]["reason"],
        format!(
            "upstream pipeline {u_id}'s last 1 runs for this change failed; \
             the next input change starts again"
        ),
        "the page's reason names U, through the chain the check walks"
    );

    let (detail_status, detail_body) =
        get_json(&app, &format!("/api/pipelines/{p_id}"), &cookie).await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(
        detail_body["lag"]["status"], "behind",
        "the detail page agrees with the list page"
    );
    assert_eq!(
        detail_body["lag"]["reason"],
        format!(
            "upstream pipeline {u_id}'s last 1 runs for this change failed; \
             the next input change starts again"
        ),
        "the detail page's reason names U through the chain, not through dependsOn"
    );
}

/// The control for the negative reading of the rule: the same shape, but
/// U's state says its retries are NOT exhausted (`request_attempt` within
/// `max_retries`, tokens still outstanding). Then P must show the plain
/// "not measured" state it owns — never the exhausted-Upstream reason.
#[tokio::test]
async fn an_outstanding_not_exhausted_upstream_does_not_make_the_page_read_behind() {
    let (app, tenant, cookie) = spin_up_with_member("lag-page-out").await;

    let u_id = create_pipeline(
        &app.pool,
        "lag-page-out-upstream",
        tenant,
        Some(TargetLag::Downstream),
        "bronze.raw",
        "silver.orders_out",
        Vec::new(),
        Some(2),
    )
    .await;
    set_ready(&app.pool, &u_id).await;
    // attempt 1 <= max_retries 2: outstanding, not exhausted.
    seed_exhausted_state(&app.pool, &u_id, tenant, 1).await;

    let p_id = create_pipeline(
        &app.pool,
        "lag-page-out-downstream",
        tenant,
        Some(TargetLag::Lag { minutes: 10 }),
        "silver.orders_out",
        "gold.p_orders",
        Vec::new(),
        None,
    )
    .await;
    set_ready(&app.pool, &p_id).await;
    seed_unmeasured_state(&app.pool, &p_id, tenant).await;

    let (status, body) = get_json(&app, "/api/pipelines", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    let row = body["pipelines"]
        .as_array()
        .expect("a pipelines array")
        .iter()
        .find(|row| row["id"].as_str() == Some(p_id.as_str()))
        .expect("the lag pipeline is listed");
    assert_eq!(
        row["lag"]["status"], "not_measured",
        "an outstanding upstream is what the check is still scheduling, not a page block"
    );
}

/// BLD-2 review R5 decision R18 / SHOULD-FIX 2: the page stops where the
/// check stops. The chain is walked deepest-first, and the check's
/// upstream evaluation answers at the FIRST upstream whose request is
/// still open — here the deeper U1 (outstanding: asked, never consumed,
/// attempts left) is named long before the nearer U2 (exhausted) is ever
/// reached. This test used to fail the other way round: the page walked
/// past every outstanding upstream to the first EXHAUSTED one, so it put
/// U2's failed-runs sentence on P while the check was saying
/// "waiting for upstream pipeline U1 to finish".
#[tokio::test]
async fn the_page_names_the_upstream_the_check_names_when_a_deeper_upstream_is_outstanding() {
    let (app, tenant, cookie) = spin_up_with_member("lag-page-two").await;

    // The deepest U1: requested but not consumed with attempts left —
    // outstanding, never exhausted (attempt 1 <= max_retries 2).
    let u1_id = create_pipeline(
        &app.pool,
        "lag-page-far-upstream",
        tenant,
        Some(TargetLag::Downstream),
        "bronze.raw",
        "silver.orders_far",
        Vec::new(),
        Some(2),
    )
    .await;
    set_ready(&app.pool, &u1_id).await;
    seed_exhausted_state(&app.pool, &u1_id, tenant, 1).await;

    // The nearer U2: exhausted (attempt 3 > max_retries 2, three failed runs).
    let u2_id = create_pipeline(
        &app.pool,
        "lag-page-near-upstream",
        tenant,
        Some(TargetLag::Downstream),
        "silver.orders_far",
        "silver.orders_near",
        Vec::new(),
        Some(2),
    )
    .await;
    set_ready(&app.pool, &u2_id).await;
    seed_exhausted_state(&app.pool, &u2_id, tenant, 3).await;

    let p_id = create_pipeline(
        &app.pool,
        "lag-page-two-downstream",
        tenant,
        Some(TargetLag::Lag { minutes: 10 }),
        "silver.orders_near",
        "gold.p_two",
        Vec::new(),
        None,
    )
    .await;
    set_ready(&app.pool, &p_id).await;
    seed_unmeasured_state(&app.pool, &p_id, tenant).await;

    let p_row = pipelines::get_pipeline(&app.pool, &p_id)
        .await
        .expect("read back P")
        .expect("P exists");
    let chain = pipelines::upstream_downstream_chain(&app.pool, &p_row)
        .await
        .expect("walk P's chain");
    assert_eq!(
        chain[0].id, u1_id,
        "the walk really is deepest-first: U1 comes before U2"
    );

    let (list_status, list_body) = get_json(&app, "/api/pipelines", &cookie).await;
    assert_eq!(list_status, StatusCode::OK);
    let row = list_body["pipelines"]
        .as_array()
        .expect("a pipelines array")
        .iter()
        .find(|row| row["id"].as_str() == Some(p_id.as_str()))
        .expect("the lag pipeline is listed");
    assert_eq!(
        row["lag"]["status"], "not_measured",
        "an outstanding first stop keeps the status derived from P's own row"
    );
    assert_eq!(
        row["lag"]["reason"],
        format!("waiting for upstream pipeline {u1_id} to finish"),
        "the page names U1, the one the check's evaluation stops at"
    );
    assert!(
        !row["lag"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("failed"),
        "U2's exhausted sentence must not leak onto P while U1 is outstanding"
    );

    let (detail_status, detail_body) =
        get_json(&app, &format!("/api/pipelines/{p_id}"), &cookie).await;
    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(
        detail_body["lag"]["reason"],
        format!("waiting for upstream pipeline {u1_id} to finish"),
        "the detail page names the same U the list page names"
    );
}
