//! Integration tests for `gold_publication.rs` (DATA-1, migration
//! `0054`): the store half of the per-mart publish-to-`Iceberg` switch.
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = "../../migrations")]` tests, backed by the
//! `lakehouse-test-support` testcontainer bootstrap — same arrangement as
//! `pipelines.rs` (see that file's header for the why).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::gold_publication;
use sqlx::PgPool;
use uuid::Uuid;

/// A mart that was never enabled reads as `None` — the "off by default"
/// state the detail route and the scheduler both rely on.
#[sqlx::test(migrations = "../../migrations")]
async fn get_is_none_for_a_mart_that_was_never_enabled(pool: PgPool) -> sqlx::Result<()> {
    let got = gold_publication::get(&pool, "never_touched")
        .await
        .expect("get on an untouched mart should succeed");
    assert!(got.is_none(), "a missing row means off, not an error");
    Ok(())
}

/// Enabling inserts a row and flipping updates it in place: one row per
/// mart, `updated_by`/`updated_at` moved by the second write, and the
/// `enabled` value the last writer chose wins.
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_inserts_then_flips_in_place(pool: PgPool) -> sqlx::Result<()> {
    let first_actor = Uuid::new_v4();

    let first = gold_publication::upsert(&pool, "orders_rollup", true, first_actor)
        .await
        .expect("first upsert should insert");
    assert_eq!(first.mart, "orders_rollup");
    assert!(first.enabled);
    assert_eq!(first.updated_by, first_actor);

    let second_actor = Uuid::new_v4();
    let second = gold_publication::upsert(&pool, "orders_rollup", false, second_actor)
        .await
        .expect("second upsert should update in place");
    assert!(!second.enabled, "the flip must reach the returned row");
    assert_eq!(second.updated_by, second_actor);
    assert!(
        second.updated_at >= first.updated_at,
        "updated_at must not go backwards: {} -> {}",
        first.updated_at,
        second.updated_at
    );

    // Exactly one row: the flip updated, it did not append.
    let got = gold_publication::get(&pool, "orders_rollup")
        .await
        .expect("get after the flip should succeed")
        .expect("the row must still exist after switching off");
    assert!(!got.enabled);
    assert_eq!(got.updated_by, second_actor);
    Ok(())
}

/// `list_enabled` returns only enabled rows — a switched-off mart stays
/// in the table but must never reach the scheduler.
#[sqlx::test(migrations = "../../migrations")]
async fn list_enabled_returns_only_enabled_rows(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    gold_publication::upsert(&pool, "mart_on_a", true, actor)
        .await
        .expect("first enable should insert");
    gold_publication::upsert(&pool, "mart_off", true, actor)
        .await
        .expect("second enable should insert");
    gold_publication::upsert(&pool, "mart_on_b", true, actor)
        .await
        .expect("third enable should insert");
    gold_publication::upsert(&pool, "mart_off", false, actor)
        .await
        .expect("the flip off should update in place");

    let enabled: Vec<String> = gold_publication::list_enabled(&pool)
        .await
        .expect("list_enabled should succeed")
        .into_iter()
        .map(|p| p.mart)
        .collect();
    assert_eq!(
        enabled,
        vec!["mart_on_a".to_owned(), "mart_on_b".to_owned()],
        "only enabled rows, ordered by mart name"
    );
    Ok(())
}

/// DATA-65: a Gold publication writes the `gold` zone by default, so a
/// pre-DATA-65 caller reading it through `get` still sees its own row.
#[sqlx::test(migrations = "../../migrations")]
async fn the_gold_helpers_still_address_the_gold_zone(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let stored = gold_publication::upsert(&pool, "orders", true, actor)
        .await
        .expect("the Gold upsert succeeds");
    assert_eq!(stored.zone, "gold");
    assert_eq!(stored.mart, "orders");

    let got = gold_publication::get(&pool, "orders")
        .await
        .expect("the Gold read succeeds")
        .expect("the row exists");
    assert_eq!(got.zone, "gold");
    assert!(got.enabled);
    Ok(())
}

/// DATA-65: `silver.orders` and `serving.orders` are two independent
/// switches. Enabling Silver does not touch the Gold row, and each
/// zone-aware read sees only its own zone.
#[sqlx::test(migrations = "../../migrations")]
async fn silver_and_gold_publications_of_the_same_mart_are_independent(
    pool: PgPool,
) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let gold = gold_publication::upsert_in_zone(&pool, "gold", "orders", true, actor)
        .await
        .expect("the Gold upsert succeeds");
    let silver = gold_publication::upsert_in_zone(&pool, "silver", "orders", true, actor)
        .await
        .expect("the Silver upsert succeeds");

    assert_eq!(gold.zone, "gold");
    assert_eq!(silver.zone, "silver");
    assert_eq!(gold.mart, "orders");
    assert_eq!(silver.mart, "orders");

    let silver_only = gold_publication::get_in_zone(&pool, "silver", "not_enabled")
        .await
        .expect("read succeeds");
    assert!(silver_only.is_none());

    // Flipping Silver off leaves Gold on.
    gold_publication::upsert_in_zone(&pool, "silver", "orders", false, actor)
        .await
        .expect("the Silver flip succeeds");
    let gold_after = gold_publication::get_in_zone(&pool, "gold", "orders")
        .await
        .expect("read succeeds")
        .expect("the Gold row is still there");
    assert!(gold_after.enabled, "the Gold switch was not touched");
    let silver_after = gold_publication::get_in_zone(&pool, "silver", "orders")
        .await
        .expect("read succeeds")
        .expect("the Silver row is still there");
    assert!(!silver_after.enabled);
    Ok(())
}

/// DATA-65: `list_enabled_in_zone` filters by zone; `list_enabled` returns
/// both zones ordered by `(zone, mart)` so the scheduler can schedule each
/// with its own zone.
#[sqlx::test(migrations = "../../migrations")]
async fn list_enabled_returns_both_zones_and_zone_queries_filter(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    gold_publication::upsert_in_zone(&pool, "gold", "zeta", true, actor)
        .await
        .expect("insert succeeds");
    gold_publication::upsert_in_zone(&pool, "silver", "alpha", true, actor)
        .await
        .expect("insert succeeds");
    gold_publication::upsert_in_zone(&pool, "gold", "alpha", true, actor)
        .await
        .expect("insert succeeds");
    gold_publication::upsert_in_zone(&pool, "silver", "off", false, actor)
        .await
        .expect("insert succeeds");

    let silver: Vec<(String, String)> = gold_publication::list_enabled_in_zone(&pool, "silver")
        .await
        .expect("list succeeds")
        .into_iter()
        .map(|p| (p.zone, p.mart))
        .collect();
    assert_eq!(silver, vec![("silver".to_owned(), "alpha".to_owned())]);

    let all: Vec<(String, String)> = gold_publication::list_enabled(&pool)
        .await
        .expect("list succeeds")
        .into_iter()
        .map(|p| (p.zone, p.mart))
        .collect();
    assert_eq!(
        all,
        vec![
            ("gold".to_owned(), "alpha".to_owned()),
            ("gold".to_owned(), "zeta".to_owned()),
            ("silver".to_owned(), "alpha".to_owned()),
        ],
        "ordered by zone then mart"
    );
    Ok(())
}

// ── DATA-66: the sync setting ─────────────────────────────────────────

/// DATA-66 D4: a row written without a mode (the pre-`0162` shape, and
/// every row `upsert_in_zone` creates) reads as `on_update` with no
/// schedule — the nearest thing to today's "after each pipeline".
#[sqlx::test(migrations = "../../migrations")]
async fn a_row_written_without_a_mode_reads_as_on_update(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let stored = gold_publication::upsert(&pool, "sync_default", true, actor)
        .await
        .expect("the insert succeeds");
    assert_eq!(stored.sync_mode, gold_publication::SyncMode::OnUpdate);
    assert_eq!(stored.schedule, None, "no schedule on the default mode");
    assert_eq!(stored.time_zone, None, "no time zone on the default mode");
    Ok(())
}

/// A scheduled setting round-trips through both readers: the enum, the
/// stored day/minute, and the IANA zone come back exactly as written.
#[sqlx::test(migrations = "../../migrations")]
async fn a_scheduled_setting_round_trips(pool: PgPool) -> sqlx::Result<()> {
    use lakehouse_store::report_schedule::Schedule;

    let actor = Uuid::new_v4();
    gold_publication::upsert(&pool, "sync_scheduled", true, actor)
        .await
        .expect("the insert succeeds");
    let setting = gold_publication::SyncSetting {
        mode: gold_publication::SyncMode::Scheduled,
        schedule: Some(Schedule::Daily { hour: 4, minute: 0 }),
        time_zone: Some("Asia/Jakarta".to_owned()),
    };
    let updated =
        gold_publication::set_sync_in_zone(&pool, "gold", "sync_scheduled", &setting, actor)
            .await
            .expect("the setting write succeeds")
            .expect("the row exists");

    assert_eq!(updated.sync_mode, gold_publication::SyncMode::Scheduled);
    assert_eq!(
        updated.schedule,
        Some(gold_publication::SyncSchedule::Daily { hour: 4, minute: 0 })
    );
    assert_eq!(updated.time_zone.as_deref(), Some("Asia/Jakarta"));

    let got = gold_publication::get(&pool, "sync_scheduled")
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_eq!(got, updated);
    Ok(())
}

/// DATA-66: flipping the switch on or off must not reset a schedule —
/// the setting has its own writer, and `upsert_in_zone` keeps its old
/// column set on conflict.
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_in_zone_keeps_a_scheduled_setting(pool: PgPool) -> sqlx::Result<()> {
    use lakehouse_store::report_schedule::Schedule;

    let actor = Uuid::new_v4();
    gold_publication::upsert(&pool, "sync_kept", true, actor)
        .await
        .expect("the insert succeeds");
    gold_publication::set_sync_in_zone(
        &pool,
        "gold",
        "sync_kept",
        &gold_publication::SyncSetting {
            mode: gold_publication::SyncMode::Scheduled,
            schedule: Some(Schedule::Hourly { minute: 15 }),
            time_zone: Some("UTC".to_owned()),
        },
        actor,
    )
    .await
    .expect("the setting write succeeds");

    let flipped = gold_publication::upsert(&pool, "sync_kept", false, Uuid::new_v4())
        .await
        .expect("the flip succeeds");
    assert!(!flipped.enabled);
    assert_eq!(flipped.sync_mode, gold_publication::SyncMode::Scheduled);
    assert_eq!(
        flipped.schedule,
        Some(gold_publication::SyncSchedule::Hourly { minute: 15 })
    );
    assert_eq!(flipped.time_zone.as_deref(), Some("UTC"));
    Ok(())
}

/// `0162`'s table CHECK refuses a `scheduled` row without a minute,
/// whatever writer made it: the API's validator and the storage layer
/// agree (fail closed at both).
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_check_refuses_scheduled_without_a_minute(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let refused = sqlx::query(
        "INSERT INTO gold_publication \
         (zone, mart, enabled, updated_by, sync_mode, sync_schedule_kind, sync_hour, sync_time_zone) \
         VALUES ('gold', 'raw_scheduled', true, $1, 'scheduled', 'daily', 4, 'UTC')",
    )
    .bind(actor)
    .execute(&pool)
    .await;
    assert!(
        refused.is_err(),
        "a scheduled row without a minute must violate the table CHECK"
    );
    let rows: (i64,) =
        sqlx::query_as("SELECT count(*) FROM gold_publication WHERE mart = 'raw_scheduled'")
            .fetch_one(&pool)
            .await?;
    assert_eq!(rows.0, 0, "the refused insert wrote no row");
    Ok(())
}

/// `set_sync_in_zone` on a row that was never switched on answers `None`:
/// there is no setting to change, and the route's own 404 is the answer.
#[sqlx::test(migrations = "../../migrations")]
async fn set_sync_in_zone_answers_none_for_a_missing_row(pool: PgPool) -> sqlx::Result<()> {
    let setting = gold_publication::SyncSetting {
        mode: gold_publication::SyncMode::Manual,
        schedule: None,
        time_zone: None,
    };
    let got = gold_publication::set_sync_in_zone(
        &pool,
        "gold",
        "never_switched_on",
        &setting,
        Uuid::new_v4(),
    )
    .await
    .expect("the update itself succeeds");
    assert!(got.is_none());
    Ok(())
}

/// Weekly/Monthly schedules and an unknown IANA zone are refused with
/// the fixed validation sentence the API maps to 400 — and nothing is
/// written.
#[sqlx::test(migrations = "../../migrations")]
async fn set_sync_in_zone_refuses_weekly_and_an_unknown_zone(pool: PgPool) -> sqlx::Result<()> {
    use lakehouse_store::StoreError;
    use lakehouse_store::report_schedule::Schedule;

    let actor = Uuid::new_v4();
    gold_publication::upsert(&pool, "sync_refused", true, actor)
        .await
        .expect("the insert succeeds");

    let weekly = gold_publication::set_sync_in_zone(
        &pool,
        "gold",
        "sync_refused",
        &gold_publication::SyncSetting {
            mode: gold_publication::SyncMode::Scheduled,
            schedule: Some(Schedule::Weekly {
                weekday: 1,
                hour: 8,
                minute: 0,
            }),
            time_zone: Some("UTC".to_owned()),
        },
        actor,
    )
    .await;
    assert!(
        matches!(weekly, Err(StoreError::Validation(message)) if message == "schedule kind must be hourly or daily")
    );

    let unknown_zone = gold_publication::set_sync_in_zone(
        &pool,
        "gold",
        "sync_refused",
        &gold_publication::SyncSetting {
            mode: gold_publication::SyncMode::Scheduled,
            schedule: Some(Schedule::Hourly { minute: 5 }),
            time_zone: Some("Mars/Olympus".to_owned()),
        },
        actor,
    )
    .await;
    assert!(
        matches!(unknown_zone, Err(StoreError::Validation(message)) if message == "unknown time zone")
    );

    let got = gold_publication::get(&pool, "sync_refused")
        .await
        .expect("the read succeeds")
        .expect("the row exists");
    assert_eq!(got.sync_mode, gold_publication::SyncMode::OnUpdate);
    Ok(())
}
