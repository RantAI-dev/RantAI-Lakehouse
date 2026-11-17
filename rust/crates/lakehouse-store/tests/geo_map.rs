//! Integration tests for `lakehouse_store::geo_map` against a real Postgres
//! (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the database is
//! provided). `BI-16` part C2b.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::geo_map::{self, NewGeoMap, delete, get, insert, list, rename};
use sqlx::PgPool;

fn new<'a>(id: &'a str, name: &'a str, file: &'a [u8]) -> NewGeoMap<'a> {
    NewGeoMap {
        id,
        name,
        name_property: "NAME",
        region_count: 2,
        geojson: file,
        created_by: "Ada",
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_map_round_trips_and_a_name_is_unique_ignoring_case(pool: PgPool) {
    let file = br#"{"type":"FeatureCollection","features":[]}"#;
    let made = insert(&pool, &new("m_0123456789ab", "Sales regions", file))
        .await
        .unwrap();
    assert_eq!(
        (made.name.as_str(), made.region_count),
        ("Sales regions", 2)
    );
    assert_eq!(list(&pool).await.unwrap().len(), 1);
    assert_eq!(
        get(&pool, "m_0123456789ab")
            .await
            .unwrap()
            .unwrap()
            .created_by,
        "Ada"
    );
    let stored = geo_map::file(&pool, "m_0123456789ab")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.geojson, file);
    assert_eq!(stored.etag.len(), 32);
    // The region lookup is off until the engine has it (migration 0102).
    assert!(!made.region_lookup);
    assert!(
        geo_map::set_region_lookup(&pool, "m_0123456789ab", true)
            .await
            .unwrap()
    );
    assert!(
        get(&pool, "m_0123456789ab")
            .await
            .unwrap()
            .unwrap()
            .region_lookup
    );
    let twin = insert(&pool, &new("m_ba9876543210", "SALES REGIONS", file)).await;
    assert!(matches!(twin, Err(StoreError::Conflict)));
    let renamed = rename(&pool, "m_0123456789ab", "Regions")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.name, "Regions");
    assert!(
        rename(&pool, "m_aaaaaaaaaaaa", "x")
            .await
            .unwrap()
            .is_none()
    );
    assert!(delete(&pool, "m_0123456789ab").await.unwrap());
    assert!(!delete(&pool, "m_0123456789ab").await.unwrap());
    assert!(get(&pool, "m_0123456789ab").await.unwrap().is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_an_id_that_is_not_server_made_and_a_file_over_5_mb(pool: PgPool) {
    let file = b"{}";
    assert!(insert(&pool, &new("not-an-id", "A", file)).await.is_err());
    assert!(
        insert(&pool, &new("m_0123456789AB", "A", file))
            .await
            .is_err()
    );
    let big = vec![b'a'; 5 * 1024 * 1024 + 1];
    assert!(
        insert(&pool, &new("m_0123456789ab", "A", &big))
            .await
            .is_err()
    );
}

/// BI-16 C3 review S4: an upload that finishes after its map was deleted must
/// learn that nothing was updated.
#[sqlx::test(migrations = "../../migrations")]
async fn setting_the_region_lookup_of_a_deleted_map_says_nothing_was_updated(pool: PgPool) {
    let file = br#"{"type":"FeatureCollection","features":[]}"#;
    insert(&pool, &new("m_0123456789ab", "Sales regions", file))
        .await
        .unwrap();
    assert!(delete(&pool, "m_0123456789ab").await.unwrap());
    assert!(
        !geo_map::set_region_lookup(&pool, "m_0123456789ab", true)
            .await
            .unwrap()
    );
}
