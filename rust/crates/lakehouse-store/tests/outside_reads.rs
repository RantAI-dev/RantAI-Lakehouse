//! Integration tests for `outside_reads.rs` (DATA-5, ADR 0016): the store
//! half of the daily read counter that answers "who read this published
//! table".
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = "../../migrations")]` tests, backed by the
//! `lakehouse-test-support` testcontainer bootstrap — same arrangement as
//! `gold_publication.rs` (see that file's header for the why).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::outside_reads;
use sqlx::PgPool;
use time::Duration;
use uuid::Uuid;

/// The day `since` for a window that starts today — the shape the API's
/// consumers route asks for ("the last 7 days").
fn today() -> time::Date {
    time::OffsetDateTime::now_utc().date()
}

/// The upsert ADDS to the day's row: two reads on the same day are one
/// row with `reads = 2`, not two rows. The door counts a load per read,
/// so a counter that appended rows would need the reader to sum them
/// (which it does) but would also lose the "one row per tool per day"
/// shape the primary key promises.
#[sqlx::test(migrations = "../../migrations")]
async fn the_upsert_adds_to_the_day_row(pool: PgPool) -> sqlx::Result<()> {
    let tool = Uuid::new_v4();

    outside_reads::record_outside_read(&pool, tool, "Finance DuckDB", "gold", "orders")
        .await
        .expect("the first read must be counted");
    outside_reads::record_outside_read(&pool, tool, "Finance DuckDB", "gold", "orders")
        .await
        .expect("the second read must be counted");
    outside_reads::record_outside_read(&pool, tool, "Finance DuckDB", "gold", "orders")
        .await
        .expect("the third read must be counted");

    let rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outside_table_read_day \
         WHERE identity_id = $1 AND namespace = 'gold' AND table_name = 'orders'",
    )
    .bind(tool)
    .fetch_one(&pool)
    .await
    .expect("count the day rows");
    assert_eq!(rows, 1, "three reads on one day are one row, not three");

    let reads: i64 = sqlx::query_scalar(
        "SELECT reads FROM outside_table_read_day \
         WHERE identity_id = $1 AND namespace = 'gold' AND table_name = 'orders'",
    )
    .bind(tool)
    .fetch_one(&pool)
    .await
    .expect("read the day's counter");
    assert_eq!(reads, 3, "each read adds one to the day's counter");

    // A different table is a different counter, and a different tool is a
    // different counter: the key is (day, tool, namespace, table).
    let other_tool = Uuid::new_v4();
    outside_reads::record_outside_read(&pool, other_tool, "BI Tool", "gold", "orders")
        .await
        .expect("another tool's read must be counted");
    let readers = outside_reads::outside_readers_of(&pool, "gold", "orders", today())
        .await
        .expect("read the readers");
    assert_eq!(readers.len(), 2, "two tools read the table");

    Ok(())
}

/// The reader SUMS the daily rows over the window it is asked about, and
/// orders the list by reads descending — the most-read tool first. A
/// read from outside the window is not counted.
#[sqlx::test(migrations = "../../migrations")]
async fn the_window_sum_and_its_order(pool: PgPool) -> sqlx::Result<()> {
    let quiet = Uuid::new_v4();
    let busy = Uuid::new_v4();
    let stale = Uuid::new_v4();

    for _ in 0..2 {
        outside_reads::record_outside_read(&pool, quiet, "Quiet Tool", "gold", "orders")
            .await
            .expect("a quiet tool's read must be counted");
    }
    for _ in 0..5 {
        outside_reads::record_outside_read(&pool, busy, "Busy Tool", "gold", "orders")
            .await
            .expect("a busy tool's read must be counted");
    }
    // A read from before the window: written straight into the table so
    // the test controls the day, and it must not be counted in a window
    // that starts today.
    sqlx::query(
        "INSERT INTO outside_table_read_day \
           (day, identity_id, identity_name, namespace, table_name, reads, last_read_at) \
         VALUES ($1, $2, 'Stale Tool', 'gold', 'orders', 99, now())",
    )
    .bind(today() - Duration::days(8))
    .bind(stale)
    .execute(&pool)
    .await
    .expect("seed a read from outside the window");

    let readers = outside_reads::outside_readers_of(&pool, "gold", "orders", today())
        .await
        .expect("read the readers");

    let names: Vec<&str> = readers.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["Busy Tool", "Quiet Tool"],
        "the list is ordered by reads descending, and a read from before the \
         window is not in it"
    );
    assert_eq!(readers[0].reads, 5);
    assert_eq!(readers[1].reads, 2);

    // A wider window does count the stale read, and the sum covers every
    // day in it.
    let wider =
        outside_reads::outside_readers_of(&pool, "gold", "orders", today() - Duration::days(30))
            .await
            .expect("read the readers over a wider window");
    let stale_reader = wider
        .iter()
        .find(|r| r.identity_id == stale)
        .expect("the stale tool must be in the wider window");
    assert_eq!(stale_reader.reads, 99);
    let busy_reader = wider
        .iter()
        .find(|r| r.identity_id == busy)
        .expect("the busy tool must be in the wider window");
    assert_eq!(busy_reader.reads, 5);

    // Another table is a different counter entirely.
    assert!(
        outside_reads::outside_readers_of(&pool, "gold", "other_mart", today())
            .await
            .expect("read the readers of another table")
            .is_empty(),
        "a tool that read one table did not read another"
    );

    Ok(())
}

/// A DELETED tool's past reads stay readable, under the name it had. The
/// counter stores the name and keeps no foreign key to the identity
/// (ADR 0016 decision 5), so deleting the identity row leaves the
/// history intact — and says who read it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_identitys_name_stays(pool: PgPool) -> sqlx::Result<()> {
    let tool = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO service_identity (id, name, scopes, environment, expires_at) \
         VALUES ($1, 'Finance DuckDB', ARRAY['iceberg:read'], 'production', now() + interval '1 day')",
    )
    .bind(tool)
    .execute(&pool)
    .await
    .expect("create the service identity");

    outside_reads::record_outside_read(&pool, tool, "Finance DuckDB", "gold", "orders")
        .await
        .expect("the read must be counted");

    sqlx::query("DELETE FROM service_identity WHERE id = $1")
        .bind(tool)
        .execute(&pool)
        .await
        .expect("delete the service identity");

    let readers = outside_reads::outside_readers_of(&pool, "gold", "orders", today())
        .await
        .expect("read the readers");
    assert_eq!(readers.len(), 1, "the deleted tool's read stays listed");
    assert_eq!(readers[0].identity_id, tool);
    assert_eq!(
        readers[0].name, "Finance DuckDB",
        "the name at the time of the read stays readable after the identity is gone"
    );
    assert_eq!(readers[0].reads, 1);
    assert!(
        !readers[0].last_read_at.is_empty(),
        "the last read is reported, not a guess"
    );

    Ok(())
}

/// The 90-day retention: rows older than the window are deleted, rows
/// inside it are kept. ADR 0016 decision 5: "Rows older than 90 days are
/// deleted."
#[sqlx::test(migrations = "../../migrations")]
async fn the_prune_removes_only_rows_older_than_the_window(pool: PgPool) -> sqlx::Result<()> {
    let old = Uuid::new_v4();
    let recent = Uuid::new_v4();
    for (tool, day) in [
        (
            old,
            today() - Duration::days(i64::from(outside_reads::RETENTION_DAYS)) - Duration::days(1),
        ),
        (
            recent,
            today() - Duration::days(i64::from(outside_reads::RETENTION_DAYS)),
        ),
    ] {
        sqlx::query(
            "INSERT INTO outside_table_read_day \
               (day, identity_id, identity_name, namespace, table_name, reads, last_read_at) \
             VALUES ($1, $2, 'Tool', 'gold', 'orders', 1, now())",
        )
        .bind(day)
        .bind(tool)
        .execute(&pool)
        .await
        .expect("seed a day row");
    }

    let deleted = outside_reads::prune_outside_reads(&pool, outside_reads::RETENTION_DAYS)
        .await
        .expect("prune the history");
    assert_eq!(deleted, 1, "only the row older than the window is deleted");

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM outside_table_read_day")
        .fetch_one(&pool)
        .await
        .expect("count the rows left");
    assert_eq!(left, 1, "the row on the window's edge is kept");

    Ok(())
}
