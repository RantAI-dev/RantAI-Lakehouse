//! BLD-11 F1: `last_run_at` used to be read by the list and the masthead
//! while nothing ever wrote it, so every authored pipeline said "Never run".
//! These tests pin the store half of the fix: the monotonic write (D2) that
//! `run_finished_event`/`run_failed_event` call through `record_last_run`.
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = "../../migrations")]` gives each test a fresh,
//! isolated database; the server is started once per binary by
//! `lakehouse-test-support` (Docker must be reachable). See `tests/pipelines.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::pipelines::{self, CreatePipelineInput};
use sqlx::PgPool;
use time::OffsetDateTime;

fn input() -> CreatePipelineInput {
    CreatePipelineInput {
        inputs: Vec::new(),
        steps: Vec::new(),
        name: "orders_hourly_rollup".to_owned(),
        kind: "incremental".to_owned(),
        source_zone: "bronze".to_owned(),
        source_table: "orders".to_owned(),
        incremental_column: None,
        transforms: Vec::new(),
        fbic_enabled: false,
        target_zone: "silver".to_owned(),
        target_table: "orders_clean".to_owned(),
        schedule: "manual".to_owned(),
        owner: None,
        description: None,
        max_retries: None,
        tenant_id: None,
        depends_on: Vec::new(),
        connector_id: None,
        target_lag: None,
        run_as: None,
        id: None,
    }
}

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

async fn create(pool: &PgPool) -> String {
    pipelines::create_pipeline(pool, &input(), None)
        .await
        .expect("create_pipeline should succeed")
        .id
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_finished_run_sets_last_run_at_on_a_pipeline_that_never_ran(pool: PgPool) {
    let id = create(&pool).await;
    let before = pipelines::get_pipeline(&pool, &id)
        .await
        .expect("get_pipeline should succeed")
        .expect("the pipeline should exist");
    assert_eq!(
        before.last_run_at, None,
        "a freshly authored pipeline never ran"
    );

    let ran_at = OffsetDateTime::from_unix_timestamp(1_700_000_000)
        .expect("a fixed unix timestamp is in range");
    let changed = pipelines::record_last_run(&pool, &id, ran_at)
        .await
        .expect("record_last_run should succeed");
    assert!(changed, "the first finished run changes the row");

    let after = pipelines::get_pipeline(&pool, &id)
        .await
        .expect("get_pipeline should succeed")
        .expect("the pipeline should exist");
    assert_eq!(after.last_run_at, Some(iso_millis(ran_at)));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_older_run_reported_late_does_not_move_last_run_at_back(pool: PgPool) {
    let id = create(&pool).await;
    let later = OffsetDateTime::from_unix_timestamp(1_700_000_100)
        .expect("a fixed unix timestamp is in range");
    let earlier = OffsetDateTime::from_unix_timestamp(1_700_000_000)
        .expect("a fixed unix timestamp is in range");

    assert!(
        pipelines::record_last_run(&pool, &id, later)
            .await
            .expect("record_last_run should succeed")
    );
    let changed = pipelines::record_last_run(&pool, &id, earlier)
        .await
        .expect("record_last_run should succeed");
    assert!(!changed, "a late older event must not move the field back");

    let after = pipelines::get_pipeline(&pool, &id)
        .await
        .expect("get_pipeline should succeed")
        .expect("the pipeline should exist");
    assert_eq!(after.last_run_at, Some(iso_millis(later)));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_pipeline_id_changes_no_row(pool: PgPool) {
    let ran_at = OffsetDateTime::from_unix_timestamp(1_700_000_000)
        .expect("a fixed unix timestamp is in range");
    let changed = pipelines::record_last_run(&pool, "pl-does-not-exist", ran_at)
        .await
        .expect("record_last_run should succeed");
    assert!(!changed, "no row means no change");
}
