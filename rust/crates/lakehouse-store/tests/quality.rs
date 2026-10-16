//! Integration tests for `lakehouse_store::quality` (`DATA-14`) against a
//! real Postgres, one freshly migrated database per test (see
//! `tests/reporting_settings.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::quality::{
    IncidentKind, Resolution, ScheduleMode, StatusFilter, freshness_resolved_since, list,
    list_schedules, load_summaries, load_summary, purge_loads, put_schedule, recent_loads,
    record_failure, record_load, resolve, resolve_for_rule,
};
use sqlx::PgPool;
use uuid::Uuid;

const USER: Uuid = Uuid::from_u128(0x3333_3333_3333_4333_8333_0000_0000_0001);

#[sqlx::test(migrations = "../../migrations")]
async fn a_failure_opens_an_incident_and_seeing_it_again_does_not_open_another(
    pool: PgPool,
) -> sqlx::Result<()> {
    let first = record_failure(
        &pool,
        "Silver.Orders",
        Some("r1"),
        IncidentKind::Check,
        "75% filled",
    )
    .await
    .unwrap();
    assert!(first.opened);
    let again = record_failure(
        &pool,
        "silver.orders",
        Some("r1"),
        IncidentKind::Check,
        "70% filled",
    )
    .await
    .unwrap();
    assert!(!again.opened);
    assert_eq!(again.id, first.id);

    let open = list(&pool, StatusFilter::Open, None).await.unwrap();
    assert_eq!(open.len(), 1);
    // The asset is kept lower-cased and the detail is the latest finding.
    assert_eq!(open[0].asset, "silver.orders");
    assert_eq!(open[0].detail, "70% filled");
    assert!(open[0].last_seen_at >= open[0].opened_at);
    assert!(open[0].opened_at.ends_with('Z'));

    // Another rule on the same table, and another table, are their own incidents.
    assert!(
        record_failure(&pool, "silver.orders", Some("r2"), IncidentKind::Check, "x")
            .await
            .unwrap()
            .opened
    );
    assert!(
        record_failure(&pool, "silver.other", Some("r1"), IncidentKind::Check, "x")
            .await
            .unwrap()
            .opened
    );
    assert_eq!(
        list(&pool, StatusFilter::Open, None).await.unwrap().len(),
        3
    );
    assert_eq!(
        list(&pool, StatusFilter::Open, Some("SILVER.ORDERS"))
            .await
            .unwrap()
            .len(),
        2
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_anomaly_is_one_incident_per_table_and_kind(pool: PgPool) -> sqlx::Result<()> {
    let volume = record_failure(
        &pool,
        "silver.orders",
        None,
        IncidentKind::VolumeAnomaly,
        "10%",
    )
    .await
    .unwrap();
    assert!(volume.opened);
    assert!(
        !record_failure(
            &pool,
            "silver.orders",
            None,
            IncidentKind::VolumeAnomaly,
            "9%"
        )
        .await
        .unwrap()
        .opened
    );
    assert!(
        record_failure(
            &pool,
            "silver.orders",
            None,
            IncidentKind::FreshnessAnomaly,
            "late"
        )
        .await
        .unwrap()
        .opened
    );
    // A check names its rule and an anomaly names none.
    assert!(matches!(
        record_failure(&pool, "silver.orders", None, IncidentKind::Check, "x").await,
        Err(StoreError::Validation(_))
    ));
    assert!(matches!(
        record_failure(
            &pool,
            "silver.orders",
            Some("r1"),
            IncidentKind::VolumeAnomaly,
            "x"
        )
        .await,
        Err(StoreError::Validation(_))
    ));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn resolving_needs_a_reason_and_frees_the_slot_for_a_new_incident(
    pool: PgPool,
) -> sqlx::Result<()> {
    let opened = record_failure(
        &pool,
        "silver.orders",
        Some("r1"),
        IncidentKind::Check,
        "bad",
    )
    .await
    .unwrap();

    for reason in ["", "   "] {
        assert!(matches!(
            resolve(&pool, &opened.id, USER, reason).await,
            Err(StoreError::Validation(_))
        ));
    }
    let too_long = "x".repeat(501);
    assert!(matches!(
        resolve(&pool, &opened.id, USER, &too_long).await,
        Err(StoreError::Validation(_))
    ));
    assert_eq!(
        list(&pool, StatusFilter::Open, None).await.unwrap().len(),
        1
    );

    let Resolution::Resolved(done) = resolve(&pool, &opened.id, USER, " fixed upstream ")
        .await
        .unwrap()
    else {
        panic!("an open incident resolves");
    };
    assert_eq!(done.status, "resolved");
    assert_eq!(done.resolution.as_deref(), Some("fixed upstream"));
    assert!(done.resolved_at.is_some());
    // The seeded user's name is joined in.
    assert!(done.resolved_by.is_some());

    assert_eq!(
        resolve(&pool, &opened.id, USER, "again").await.unwrap(),
        Resolution::AlreadyResolved
    );
    assert_eq!(
        resolve(&pool, &Uuid::new_v4().to_string(), USER, "x")
            .await
            .unwrap(),
        Resolution::NotFound
    );
    assert_eq!(
        resolve(&pool, "not-a-uuid", USER, "x").await.unwrap(),
        Resolution::NotFound
    );

    // The same failure again is a new incident; the old one stays as history.
    let reopened = record_failure(
        &pool,
        "silver.orders",
        Some("r1"),
        IncidentKind::Check,
        "bad",
    )
    .await
    .unwrap();
    assert!(reopened.opened);
    assert_ne!(reopened.id, opened.id);
    assert_eq!(list(&pool, StatusFilter::All, None).await.unwrap().len(), 2);
    assert_eq!(
        list(&pool, StatusFilter::Resolved, None)
            .await
            .unwrap()
            .len(),
        1
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn resolving_a_rules_incidents_closes_only_that_rules_open_ones(
    pool: PgPool,
) -> sqlx::Result<()> {
    record_failure(&pool, "silver.orders", Some("r1"), IncidentKind::Check, "a")
        .await
        .unwrap();
    record_failure(&pool, "silver.orders", Some("r2"), IncidentKind::Check, "b")
        .await
        .unwrap();
    record_failure(
        &pool,
        "silver.orders",
        None,
        IncidentKind::VolumeAnomaly,
        "c",
    )
    .await
    .unwrap();

    assert_eq!(
        resolve_for_rule(&pool, "r1", USER, "rule deleted")
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        resolve_for_rule(&pool, "r1", USER, "rule deleted")
            .await
            .unwrap(),
        0
    );
    let open = list(&pool, StatusFilter::Open, None).await.unwrap();
    assert_eq!(open.len(), 2);
    let resolved = list(&pool, StatusFilter::Resolved, None).await.unwrap();
    assert_eq!(resolved[0].resolution.as_deref(), Some("rule deleted"));
    Ok(())
}

/// A very long finding is cut to the table's limit rather than refused: the
/// run that found it must not fail over its own bookkeeping.
#[sqlx::test(migrations = "../../migrations")]
async fn a_long_detail_is_clipped(pool: PgPool) -> sqlx::Result<()> {
    let long = "y".repeat(5000);
    record_failure(
        &pool,
        "silver.orders",
        Some("r1"),
        IncidentKind::Check,
        &long,
    )
    .await
    .unwrap();
    let open = list(&pool, StatusFilter::Open, None).await.unwrap();
    assert_eq!(open[0].detail.chars().count(), 1000);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_schedule_is_kept_per_table_and_replaced_by_the_next_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    // No row is the default: nothing listed.
    assert!(list_schedules(&pool).await.unwrap().is_empty());

    let first = put_schedule(&pool, " Silver.Orders ", ScheduleMode::Interval, 180, USER)
        .await
        .unwrap();
    assert_eq!(first.asset, "silver.orders");
    assert_eq!(first.mode, "interval");
    assert_eq!(first.interval_minutes, 180);
    assert!(first.updated_by.is_some());
    assert!(first.updated_at.ends_with('Z'));

    let second = put_schedule(&pool, "silver.orders", ScheduleMode::OnLoad, 1440, USER)
        .await
        .unwrap();
    assert_eq!(second.mode, "on_load");
    put_schedule(&pool, "serving.mart", ScheduleMode::Interval, 60, USER)
        .await
        .unwrap();
    let all = list_schedules(&pool).await.unwrap();
    assert_eq!(
        all.iter()
            .map(|s| (s.asset.as_str(), s.mode.as_str(), s.interval_minutes))
            .collect::<Vec<_>>(),
        vec![
            ("serving.mart", "interval", 60),
            ("silver.orders", "on_load", 1440)
        ]
    );
    Ok(())
}

/// The bounds are one hour and one day, both inclusive.
#[sqlx::test(migrations = "../../migrations")]
async fn a_schedule_interval_is_between_one_hour_and_one_day(pool: PgPool) -> sqlx::Result<()> {
    for minutes in [-1, 0, 30, 59, 1441, 100_000] {
        assert!(
            matches!(
                put_schedule(
                    &pool,
                    "silver.orders",
                    ScheduleMode::Interval,
                    minutes,
                    USER
                )
                .await,
                Err(StoreError::Validation(_))
            ),
            "{minutes}"
        );
    }
    for minutes in [60, 61, 1439, 1440] {
        put_schedule(
            &pool,
            "silver.orders",
            ScheduleMode::Interval,
            minutes,
            USER,
        )
        .await
        .unwrap();
    }
    assert!(matches!(
        put_schedule(&pool, "  ", ScheduleMode::Interval, 60, USER).await,
        Err(StoreError::Validation(_))
    ));
    Ok(())
}

async fn backdated_load(pool: &PgPool, asset: &str, days_ago: i32, rows: i64) {
    sqlx::query(
        "INSERT INTO table_load (asset, loaded_at, rows_after) \
         VALUES ($1, now() - make_interval(days => $2), $3)",
    )
    .bind(asset)
    .bind(days_ago)
    .bind(rows)
    .execute(pool)
    .await
    .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn loads_are_recorded_per_table_and_read_back_oldest_first(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        load_summary(&pool, "silver.orders")
            .await
            .unwrap()
            .is_none()
    );
    for (days_ago, rows) in [(5, 100), (3, 200), (1, 300)] {
        backdated_load(&pool, "silver.orders", days_ago, rows).await;
    }
    backdated_load(&pool, "silver.other", 2, 9).await;
    record_load(&pool, " Silver.Orders ", 400).await.unwrap();

    let all = recent_loads(&pool, "silver.orders", 50).await.unwrap();
    assert_eq!(
        all.iter().map(|l| l.rows_after).collect::<Vec<_>>(),
        vec![100, 200, 300, 400]
    );
    // The newest two, still oldest first.
    let newest = recent_loads(&pool, "silver.orders", 2).await.unwrap();
    assert_eq!(
        newest.iter().map(|l| l.rows_after).collect::<Vec<_>>(),
        vec![300, 400]
    );

    let summary = load_summary(&pool, "SILVER.ORDERS").await.unwrap().unwrap();
    assert_eq!(summary.loads, 4);
    assert!(summary.first_at < summary.last_at);
    let every = load_summaries(&pool).await.unwrap();
    assert_eq!(
        every
            .iter()
            .map(|s| (s.asset.as_str(), s.loads))
            .collect::<Vec<_>>(),
        vec![("silver.orders", 4), ("silver.other", 1)]
    );

    // A count below zero is refused by the table, not stored.
    assert!(record_load(&pool, "silver.orders", -1).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn loads_older_than_ninety_days_are_purged(pool: PgPool) -> sqlx::Result<()> {
    backdated_load(&pool, "silver.orders", 91, 1).await;
    backdated_load(&pool, "silver.orders", 90, 2).await;
    backdated_load(&pool, "silver.orders", 89, 3).await;
    assert_eq!(purge_loads(&pool).await.unwrap(), 2);
    let kept = recent_loads(&pool, "silver.orders", 10).await.unwrap();
    assert_eq!(
        kept.iter().map(|l| l.rows_after).collect::<Vec<_>>(),
        vec![3]
    );
    Ok(())
}

/// Only a RESOLVED freshness incident opened after the table's last load
/// suppresses a new one.
#[sqlx::test(migrations = "../../migrations")]
async fn a_resolved_freshness_incident_after_the_last_load_is_remembered(
    pool: PgPool,
) -> sqlx::Result<()> {
    let last_load = time::OffsetDateTime::now_utc() - time::Duration::hours(1);
    assert!(
        !freshness_resolved_since(&pool, "silver.orders", last_load)
            .await
            .unwrap()
    );

    let opened = record_failure(
        &pool,
        "silver.orders",
        None,
        IncidentKind::FreshnessAnomaly,
        "late",
    )
    .await
    .unwrap();
    // Open is not resolved.
    assert!(
        !freshness_resolved_since(&pool, "silver.orders", last_load)
            .await
            .unwrap()
    );
    resolve(&pool, &opened.id, USER, "known").await.unwrap();
    assert!(
        freshness_resolved_since(&pool, "silver.orders", last_load)
            .await
            .unwrap()
    );
    // A load after the incident opened ends that silence.
    let later = time::OffsetDateTime::now_utc() + time::Duration::hours(1);
    assert!(
        !freshness_resolved_since(&pool, "silver.orders", later)
            .await
            .unwrap()
    );
    // Another table, and another kind, do not count.
    assert!(
        !freshness_resolved_since(&pool, "silver.other", last_load)
            .await
            .unwrap()
    );
    let volume = record_failure(
        &pool,
        "silver.vol",
        None,
        IncidentKind::VolumeAnomaly,
        "small",
    )
    .await
    .unwrap();
    resolve(&pool, &volume.id, USER, "known").await.unwrap();
    assert!(
        !freshness_resolved_since(&pool, "silver.vol", last_load)
            .await
            .unwrap()
    );
    Ok(())
}
