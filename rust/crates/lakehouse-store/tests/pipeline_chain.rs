//! Integration tests for `lakehouse_store::pipeline_chain` against a real
//! Postgres (`BLD-14`, D8; migration `0153`). They pin the write-once key, the
//! P9 retention, the same-tenant cause scope and the table's CHECKs.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::pipeline_chain::{ChainStep, latest_chain_step, record_chain_step};
use sqlx::PgPool;
use uuid::Uuid;

async fn tenant(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO tenant (name, slug, plan, residency) \
         VALUES ($1, $1, 'team', 'eu') RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn pipeline(pool: &PgPool, id: &str, name: &str, tenant_id: Option<Uuid>) {
    sqlx::query(
        "INSERT INTO pipeline_definition \
           (id, name, kind, status, source, target, schedule, tenant_id) \
         VALUES ($1, $2, 'batch', 'ready', 'bronze.orders', 'silver.orders', '', $3)",
    )
    .bind(id)
    .bind(name)
    .bind(tenant_id)
    .execute(pool)
    .await
    .unwrap();
}

fn step(root: &str, pipeline_id: &str, outcome: &str) -> ChainStep {
    ChainStep {
        root_run_id: root.to_owned(),
        pipeline_id: pipeline_id.to_owned(),
        source_table: "bronze.orders".to_owned(),
        outcome: outcome.to_owned(),
        reason: (outcome == "not_run").then(|| "upstream_failed".to_owned()),
        run_id: (outcome == "started").then(|| format!("{root}-run")),
        cause_pipeline_id: (outcome == "not_run").then(|| "pl-cause".to_owned()),
    }
}

/// A sensor retry for the same (re-upload, pipeline) does not replace the
/// first outcome: only the first write reports `true`.
#[sqlx::test(migrations = "../../migrations")]
async fn the_second_record_for_the_same_reupload_and_pipeline_is_not_written(
    pool: PgPool,
) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    assert!(
        record_chain_step(&pool, &step("root-1", "pl-gold", "started"))
            .await
            .unwrap()
    );
    assert!(
        !record_chain_step(&pool, &step("root-1", "pl-gold", "not_run"))
            .await
            .unwrap()
    );
    let last = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(last.root_run_id, "root-1");
    assert_eq!(last.outcome, "started");
    assert_eq!(last.run_id.as_deref(), Some("root-1-run"));
    Ok(())
}

/// The page shows the step with the newest `recorded_at`, not an arbitrary
/// row from the primary key's order.
#[sqlx::test(migrations = "../../migrations")]
async fn the_newest_record_is_the_one_read(pool: PgPool) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    pipeline(&pool, "pl-cause", "silver-orders", None).await;
    record_chain_step(&pool, &step("root-1", "pl-gold", "started"))
        .await
        .unwrap();
    record_chain_step(&pool, &step("root-2", "pl-gold", "not_run"))
        .await
        .unwrap();
    sqlx::query(
        "UPDATE pipeline_chain_step SET recorded_at = recorded_at - INTERVAL '1 hour' \
         WHERE root_run_id = 'root-2'",
    )
    .execute(&pool)
    .await?;
    let last = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(last.root_run_id, "root-1");
    assert_eq!(last.outcome, "started");
    Ok(())
}

/// A cause in another tenant still keeps its id in the record, but its name
/// is not revealed to the reader (section 3).
#[sqlx::test(migrations = "../../migrations")]
async fn a_cause_in_another_tenant_has_no_name(pool: PgPool) -> sqlx::Result<()> {
    let a = tenant(&pool, "tenant-a").await;
    let b = tenant(&pool, "tenant-b").await;
    pipeline(&pool, "pl-gold", "gold-orders", Some(a)).await;
    pipeline(&pool, "pl-cause", "secret-silver", Some(b)).await;
    record_chain_step(&pool, &step("root-1", "pl-gold", "not_run"))
        .await
        .unwrap();
    let last = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(last.cause_pipeline_id.as_deref(), Some("pl-cause"));
    assert_eq!(last.cause_pipeline_name, None);
    Ok(())
}

/// `cause_pipeline_id` has no foreign key: deleting the cause does not delete
/// the record, and the read then answers no name.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_cause_keeps_the_record_with_no_name(pool: PgPool) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    pipeline(&pool, "pl-cause", "silver-orders", None).await;
    record_chain_step(&pool, &step("root-1", "pl-gold", "not_run"))
        .await
        .unwrap();
    let before = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(before.cause_pipeline_name.as_deref(), Some("silver-orders"));
    sqlx::query("DELETE FROM pipeline_definition WHERE id = 'pl-cause'")
        .execute(&pool)
        .await?;
    let after = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(after.cause_pipeline_id.as_deref(), Some("pl-cause"));
    assert_eq!(after.cause_pipeline_name, None);
    Ok(())
}

/// P9: only the newest 20 rows of a pipeline survive, in the insert's own
/// transaction.
#[sqlx::test(migrations = "../../migrations")]
async fn only_the_newest_twenty_records_of_a_pipeline_are_kept(pool: PgPool) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    for i in 0..21 {
        let root = format!("root-{i:02}");
        record_chain_step(&pool, &step(&root, "pl-gold", "started"))
            .await
            .unwrap();
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pipeline_chain_step WHERE pipeline_id = 'pl-gold'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 20);
    let last = latest_chain_step(&pool, "pl-gold")
        .await
        .unwrap()
        .expect("a row");
    assert_eq!(last.root_run_id, "root-20");
    Ok(())
}

/// The record's foreign key cascades with the pipeline: no orphan rows are
/// left for the page to read.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_the_pipeline_deletes_its_records(pool: PgPool) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    record_chain_step(&pool, &step("root-1", "pl-gold", "started"))
        .await
        .unwrap();
    assert!(latest_chain_step(&pool, "pl-gold").await.unwrap().is_some());
    sqlx::query("DELETE FROM pipeline_definition WHERE id = 'pl-gold'")
        .execute(&pool)
        .await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pipeline_chain_step WHERE pipeline_id = 'pl-gold'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 0);
    Ok(())
}

/// The table, not just the route, refuses an incoherent outcome: `not_run`
/// without a reason and `started` without a run id.
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_not_run_without_a_reason(pool: PgPool) -> sqlx::Result<()> {
    pipeline(&pool, "pl-gold", "gold-orders", None).await;
    assert!(
        sqlx::query(
            "INSERT INTO pipeline_chain_step \
               (root_run_id, pipeline_id, source_table, outcome) \
             VALUES ('root-1', 'pl-gold', 'bronze.orders', 'not_run')",
        )
        .execute(&pool)
        .await
        .is_err()
    );
    assert!(
        sqlx::query(
            "INSERT INTO pipeline_chain_step \
               (root_run_id, pipeline_id, source_table, outcome) \
             VALUES ('root-2', 'pl-gold', 'bronze.orders', 'started')",
        )
        .execute(&pool)
        .await
        .is_err()
    );
    Ok(())
}
