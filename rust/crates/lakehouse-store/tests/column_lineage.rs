//! Integration tests for `lakehouse_store::column_lineage` (`DATA-13`)
//! against a real Postgres, one freshly migrated database per test (see
//! `tests/reporting_settings.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::column_lineage::{
    Direction, NewMapping, Scope, builds_cursor, clear_pending, delete_producer, get_pending,
    oldest_recorded_at, purge_expired, put_pending, replace_producer, set_builds_cursor,
    touching_table, walk,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

fn map(from: (&str, &str), to: (&str, &str), transform: &str) -> NewMapping {
    NewMapping {
        from_table: Some(from.0.to_owned()),
        from_column: Some(from.1.to_owned()),
        to_table: to.0.to_owned(),
        to_column: to.1.to_owned(),
        transform: transform.to_owned(),
        partial: false,
        reason: None,
    }
}

fn partial(to: (&str, &str), reason: &str) -> NewMapping {
    NewMapping {
        from_table: None,
        from_column: None,
        to_table: to.0.to_owned(),
        to_column: to.1.to_owned(),
        transform: "unknown".to_owned(),
        partial: true,
        reason: Some(reason.to_owned()),
    }
}

fn everything() -> Scope {
    Scope {
        tenant: None,
        all_tenants: true,
        shared: true,
    }
}

fn tenant_only(t: Uuid) -> Scope {
    Scope {
        tenant: Some(t),
        all_tenants: false,
        shared: false,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_producer_is_replaced_as_a_set(pool: PgPool) -> sqlx::Result<()> {
    let first = [
        map(("bronze.a", "x"), ("silver.a", "x"), "copy"),
        map(("bronze.a", "y"), ("silver.a", "y2"), "copy"),
    ];
    replace_producer(&pool, "pipeline", "p1", None, &first)
        .await
        .expect("first");
    let second = [map(("bronze.a", "z"), ("silver.a", "z"), "cast")];
    replace_producer(&pool, "pipeline", "p1", None, &second)
        .await
        .expect("second");

    let rows = touching_table(&pool, "silver.a", everything())
        .await
        .expect("touching");
    assert_eq!(rows.len(), 1, "the earlier set is gone");
    assert_eq!(rows[0].from_column.as_deref(), Some("z"));
    assert_eq!(rows[0].transform, "cast");

    replace_producer(&pool, "pipeline", "p1", None, &[])
        .await
        .expect("empty");
    assert!(
        touching_table(&pool, "silver.a", everything())
            .await
            .expect("touching")
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn replacing_one_producer_leaves_another_alone(pool: PgPool) -> sqlx::Result<()> {
    replace_producer(
        &pool,
        "pipeline",
        "p1",
        None,
        &[map(("bronze.a", "x"), ("silver.a", "x"), "copy")],
    )
    .await
    .expect("p1");
    replace_producer(
        &pool,
        "build",
        "serving.m",
        None,
        &[map(("silver.a", "x"), ("serving.m", "x"), "copy")],
    )
    .await
    .expect("build");
    replace_producer(&pool, "pipeline", "p1", None, &[])
        .await
        .expect("clear p1");
    let rows = touching_table(&pool, "serving.m", everything())
        .await
        .expect("touching");
    assert_eq!(rows.len(), 1);
    assert_eq!(
        delete_producer(&pool, "build", "serving.m").await.unwrap(),
        1
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_walk_up_follows_mappings_across_producers_with_hops(pool: PgPool) -> sqlx::Result<()> {
    replace_producer(
        &pool,
        "pipeline",
        "p1",
        None,
        &[map(("bronze.a", "x"), ("silver.a", "x1"), "copy")],
    )
    .await
    .unwrap();
    replace_producer(
        &pool,
        "build",
        "serving.m",
        None,
        &[map(
            ("silver.a", "x1"),
            ("serving.m", "total"),
            "expression",
        )],
    )
    .await
    .unwrap();

    let up = walk(&pool, "serving.m", "total", Direction::Up, everything())
        .await
        .unwrap();
    let hops: Vec<(String, i32)> = up
        .iter()
        .map(|r| (r.from_table.clone().unwrap(), r.hop))
        .collect();
    assert_eq!(
        hops,
        vec![("silver.a".to_owned(), 1), ("bronze.a".to_owned(), 2)]
    );

    let down = walk(&pool, "bronze.a", "x", Direction::Down, everything())
        .await
        .unwrap();
    let ends: Vec<(String, i32)> = down.iter().map(|r| (r.to_table.clone(), r.hop)).collect();
    assert_eq!(
        ends,
        vec![("silver.a".to_owned(), 1), ("serving.m".to_owned(), 2)]
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_walk_stops_at_ten_hops_and_survives_a_cycle(pool: PgPool) -> sqlx::Result<()> {
    // A chain of 12 tables, plus an edge that closes it into a loop.
    let mut rows = Vec::new();
    for i in 0..12 {
        rows.push(map(
            (&format!("silver.t{i}"), "c"),
            (&format!("silver.t{}", (i + 1) % 12), "c"),
            "copy",
        ));
    }
    replace_producer(&pool, "build", "loop", None, &rows)
        .await
        .unwrap();
    let up = walk(&pool, "silver.t0", "c", Direction::Up, everything())
        .await
        .unwrap();
    assert_eq!(up.len(), 10, "ten hops of a ring are ten distinct rows");
    assert_eq!(up.iter().map(|r| r.hop).max(), Some(10));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_partial_row_is_returned_with_its_reason_and_ends_the_walk(
    pool: PgPool,
) -> sqlx::Result<()> {
    replace_producer(
        &pool,
        "build",
        "serving.m",
        None,
        &[partial(("serving.m", "n"), "built from a subquery")],
    )
    .await
    .unwrap();
    let up = walk(&pool, "serving.m", "n", Direction::Up, everything())
        .await
        .unwrap();
    assert_eq!(up.len(), 1);
    assert!(up[0].partial);
    assert_eq!(up[0].reason.as_deref(), Some("built from a subquery"));
    assert_eq!(up[0].from_column, None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_partial_row_without_a_reason_and_an_unnamed_complete_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    let no_reason = NewMapping {
        reason: None,
        ..partial(("silver.a", "x"), "unused")
    };
    assert!(
        replace_producer(&pool, "build", "silver.a", None, &[no_reason])
            .await
            .is_err()
    );
    let unnamed = NewMapping {
        from_column: None,
        partial: false,
        ..map(("bronze.a", "x"), ("silver.a", "x"), "copy")
    };
    assert!(
        replace_producer(&pool, "build", "silver.a", None, &[unnamed])
            .await
            .is_err()
    );
    let bad_kind = replace_producer(
        &pool,
        "other",
        "x",
        None,
        &[map(("bronze.a", "x"), ("silver.a", "x"), "copy")],
    )
    .await;
    assert!(bad_kind.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_set_over_the_row_limit_is_refused_before_it_is_written(
    pool: PgPool,
) -> sqlx::Result<()> {
    let rows: Vec<NewMapping> = (0..=lakehouse_store::column_lineage::MAX_ROWS_PER_PRODUCER)
        .map(|i| map(("bronze.a", "x"), ("silver.a", &format!("c{i}")), "copy"))
        .collect();
    let result = replace_producer(&pool, "pipeline", "big", None, &rows).await;
    assert!(matches!(result, Err(StoreError::Validation(_))));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_tenant_sees_its_own_rows_and_shared_ones_only_when_allowed(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mine = Uuid::new_v4();
    let theirs = Uuid::new_v4();
    replace_producer(
        &pool,
        "pipeline",
        "mine",
        Some(mine),
        &[map(("bronze.a", "x"), ("silver.a", "x"), "copy")],
    )
    .await
    .unwrap();
    replace_producer(
        &pool,
        "pipeline",
        "theirs",
        Some(theirs),
        &[map(("bronze.b", "x"), ("silver.a", "x"), "copy")],
    )
    .await
    .unwrap();
    replace_producer(
        &pool,
        "build",
        "shared",
        None,
        &[map(("silver.a", "x"), ("serving.m", "x"), "copy")],
    )
    .await
    .unwrap();

    let own = walk(&pool, "silver.a", "x", Direction::Up, tenant_only(mine))
        .await
        .unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].from_table.as_deref(), Some("bronze.a"));

    // Without the shared rows the walk down from silver finds nothing.
    let down = walk(&pool, "silver.a", "x", Direction::Down, tenant_only(mine))
        .await
        .unwrap();
    assert!(down.is_empty());
    let with_shared = Scope {
        shared: true,
        ..tenant_only(mine)
    };
    let down = walk(&pool, "silver.a", "x", Direction::Down, with_shared)
        .await
        .unwrap();
    assert_eq!(down.len(), 1);

    let all = walk(&pool, "silver.a", "x", Direction::Up, everything())
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn purge_removes_only_rows_older_than_a_year_and_oldest_is_reported(
    pool: PgPool,
) -> sqlx::Result<()> {
    replace_producer(
        &pool,
        "build",
        "silver.old",
        None,
        &[map(("bronze.a", "x"), ("silver.old", "x"), "copy")],
    )
    .await
    .unwrap();
    replace_producer(
        &pool,
        "build",
        "silver.new",
        None,
        &[map(("bronze.a", "x"), ("silver.new", "x"), "copy")],
    )
    .await
    .unwrap();
    sqlx::query(
        "UPDATE column_lineage SET recorded_at = now() - interval '366 days' \
         WHERE producer_ref = 'silver.old'",
    )
    .execute(&pool)
    .await?;
    assert_eq!(purge_expired(&pool).await.unwrap(), 1);
    assert!(
        touching_table(&pool, "silver.old", everything())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        touching_table(&pool, "silver.new", everything())
            .await
            .unwrap()
            .len(),
        1
    );
    let oldest = oldest_recorded_at(&pool, everything()).await.unwrap();
    assert!(oldest.is_some());
    assert_eq!(
        oldest_recorded_at(&pool, tenant_only(Uuid::new_v4()))
            .await
            .unwrap(),
        None
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn pending_sql_is_kept_replaced_and_cleared(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(get_pending(&pool, "p1").await.unwrap(), None);
    put_pending(&pool, "p1", "SELECT a FROM t").await.unwrap();
    put_pending(&pool, "p1", "SELECT b FROM t").await.unwrap();
    assert_eq!(
        get_pending(&pool, "p1").await.unwrap().as_deref(),
        Some("SELECT b FROM t")
    );
    clear_pending(&pool, "p1").await.unwrap();
    assert_eq!(get_pending(&pool, "p1").await.unwrap(), None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_build_cursor_starts_empty_and_moves(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(builds_cursor(&pool).await.unwrap(), None);
    let t1 = OffsetDateTime::from_unix_timestamp(1_760_000_000).unwrap();
    let t2 = OffsetDateTime::from_unix_timestamp(1_760_000_900).unwrap();
    set_builds_cursor(&pool, t1).await.unwrap();
    set_builds_cursor(&pool, t2).await.unwrap();
    assert_eq!(builds_cursor(&pool).await.unwrap(), Some(t2));
    let (n,): (i64,) = sqlx::query_as("SELECT count(*) FROM column_lineage_state")
        .fetch_one(&pool)
        .await?;
    assert_eq!(n, 1);
    Ok(())
}
