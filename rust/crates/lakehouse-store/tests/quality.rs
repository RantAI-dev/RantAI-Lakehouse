//! Integration tests for `lakehouse_store::quality` (`DATA-14`) against a
//! real Postgres, one freshly migrated database per test (see
//! `tests/reporting_settings.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::quality::{
    IncidentKind, Resolution, StatusFilter, list, record_failure, resolve, resolve_for_rule,
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
