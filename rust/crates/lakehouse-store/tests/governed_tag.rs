//! Integration tests for `lakehouse_store::governed_tag` (`DATA-12` part 2)
//! against a real Postgres, one freshly migrated database per test (see
//! `tests/annotation.rs` for how the server is started).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::governed_tag::{
    create_key, delete_key, list_for_asset, list_for_assets, list_keys, set_asset_tags,
    update_key_values,
};
use sqlx::PgPool;

fn words(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

fn pairs(tags: &[(&str, &str)]) -> Vec<(String, String)> {
    tags.iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect()
}

/// The sentence of a `Validation` refusal; fails the test on any other error.
fn refusal(result: Result<impl std::fmt::Debug, StoreError>) -> String {
    match result {
        Err(StoreError::Validation(message)) => message,
        other => panic!("expected a Validation refusal, got {other:?}"),
    }
}

async fn domain_key(pool: &PgPool) {
    create_key(pool, "data_domain", &words(&["finance", "ops"]), "Rina")
        .await
        .expect("create key");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_key_round_trips_with_its_values_and_creator(pool: PgPool) -> sqlx::Result<()> {
    let made = create_key(&pool, "data_domain", &words(&["finance", "ops"]), "Rina")
        .await
        .expect("create");
    assert_eq!(made.key, "data_domain");
    assert_eq!(made.allowed_values, words(&["finance", "ops"]));
    assert_eq!(made.created_by, "Rina");

    let keys = list_keys(&pool).await.expect("list");
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].key, "data_domain");
    assert_eq!(keys[0].allowed_values, words(&["finance", "ops"]));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn defining_a_key_twice_is_a_conflict(pool: PgPool) -> sqlx::Result<()> {
    domain_key(&pool).await;
    let again = create_key(&pool, "data_domain", &words(&["x"]), "Rina").await;
    assert!(matches!(again, Err(StoreError::Conflict)), "{again:?}");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_value_bounds_are_refused_before_anything_is_written(pool: PgPool) -> sqlx::Result<()> {
    let fifty_one: Vec<String> = (0..51).map(|i| format!("v{i}")).collect();
    let fifty: Vec<String> = (0..50).map(|i| format!("v{i}")).collect();
    assert!(refusal(create_key(&pool, "k", &fifty_one, "Rina").await).contains("at most 50"));
    assert!(refusal(create_key(&pool, "k", &[], "Rina").await).contains("at least one"));
    assert!(
        refusal(create_key(&pool, "k", &words(&["Finance"]), "Rina").await)
            .contains("allowedValues: \"Finance\"")
    );
    assert!(
        refusal(create_key(&pool, "k", &words(&["a", "a"]), "Rina").await)
            .contains("more than once")
    );
    assert!(refusal(create_key(&pool, "Bad Key", &words(&["a"]), "Rina").await).contains("key"));
    let long = "a".repeat(65);
    assert!(refusal(create_key(&pool, &long, &words(&["a"]), "Rina").await).contains("key"));
    assert!(
        refusal(create_key(&pool, "k", &[long], "Rina").await).contains("allowedValues"),
        "a 65-character value is refused"
    );
    assert!(list_keys(&pool).await.expect("list").is_empty());

    create_key(&pool, "k", &fifty, "Rina")
        .await
        .expect("exactly 50 values is allowed");
    Ok(())
}

/// The CHECKs and the foreign key are the backstop for a caller that skips
/// the store functions, so they are exercised with raw SQL.
#[sqlx::test(migrations = "../../migrations")]
async fn the_migration_refuses_what_the_store_would_refuse(pool: PgPool) -> sqlx::Result<()> {
    let bad_key = sqlx::query(
        "INSERT INTO governed_tag_key (key, allowed_values, created_by) VALUES ('Bad', '{a}', 'x')",
    )
    .execute(&pool)
    .await;
    assert!(bad_key.is_err(), "an uppercase key is refused by a CHECK");

    let bad_value = sqlx::query(
        "INSERT INTO governed_tag_key (key, allowed_values, created_by) VALUES ('k', '{Bad}', 'x')",
    )
    .execute(&pool)
    .await;
    assert!(
        bad_value.is_err(),
        "an uppercase value is refused by a CHECK"
    );

    domain_key(&pool).await;
    let orphan = sqlx::query(
        "INSERT INTO asset_governed_tag (asset_id, tag_key, value, set_by) \
         VALUES ('silver.a', 'no_such_key', 'x', 'x')",
    )
    .execute(&pool)
    .await;
    assert!(orphan.is_err(), "a pair needs a defined key");

    set_asset_tags(
        &pool,
        "silver.a",
        &pairs(&[("data_domain", "finance")]),
        "Rina",
    )
    .await
    .expect("tag");
    let dropped = sqlx::query("DELETE FROM governed_tag_key WHERE key = 'data_domain'")
        .execute(&pool)
        .await;
    assert!(dropped.is_err(), "ON DELETE RESTRICT holds the key");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn setting_tags_replaces_the_whole_set(pool: PgPool) -> sqlx::Result<()> {
    domain_key(&pool).await;
    create_key(&pool, "tier", &words(&["gold", "bronze"]), "Rina")
        .await
        .expect("second key");

    set_asset_tags(
        &pool,
        "serving.mart",
        &pairs(&[("data_domain", "finance"), ("tier", "gold")]),
        "Rina",
    )
    .await
    .expect("first set");
    let first = list_for_asset(&pool, "serving.mart").await.expect("list");
    let got: Vec<_> = first
        .iter()
        .map(|r| (r.tag_key.as_str(), r.value.as_str()))
        .collect();
    assert_eq!(got, vec![("data_domain", "finance"), ("tier", "gold")]);

    set_asset_tags(
        &pool,
        "serving.mart",
        &pairs(&[("data_domain", "ops")]),
        "Dian",
    )
    .await
    .expect("second set");
    let second = list_for_asset(&pool, "serving.mart").await.expect("list");
    assert_eq!(second.len(), 1, "the pair left out is removed");
    assert_eq!(second[0].value, "ops");
    assert_eq!(second[0].set_by, "Dian");

    set_asset_tags(&pool, "serving.mart", &[], "Dian")
        .await
        .expect("clear");
    assert!(
        list_for_asset(&pool, "serving.mart")
            .await
            .expect("list")
            .is_empty()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_pair_that_did_not_change_keeps_who_set_it(pool: PgPool) -> sqlx::Result<()> {
    domain_key(&pool).await;
    create_key(&pool, "tier", &words(&["gold", "bronze"]), "Rina")
        .await
        .expect("second key");
    set_asset_tags(
        &pool,
        "silver.a",
        &pairs(&[("data_domain", "finance")]),
        "Rina",
    )
    .await
    .expect("first");
    let before = list_for_asset(&pool, "silver.a").await.expect("list");

    set_asset_tags(
        &pool,
        "silver.a",
        &pairs(&[("data_domain", "finance"), ("tier", "gold")]),
        "Dian",
    )
    .await
    .expect("second");
    let after = list_for_asset(&pool, "silver.a").await.expect("list");
    let domain = after.iter().find(|r| r.tag_key == "data_domain").unwrap();
    assert_eq!(domain.set_by, "Rina", "unchanged pair keeps its setter");
    assert_eq!(domain.set_at, before[0].set_at);
    let tier = after.iter().find(|r| r.tag_key == "tier").unwrap();
    assert_eq!(tier.set_by, "Dian");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_value_outside_the_list_is_refused_naming_the_key_and_the_allowed_values(
    pool: PgPool,
) -> sqlx::Result<()> {
    domain_key(&pool).await;
    set_asset_tags(
        &pool,
        "silver.a",
        &pairs(&[("data_domain", "finance")]),
        "Rina",
    )
    .await
    .expect("seed");

    let message = refusal(
        set_asset_tags(
            &pool,
            "silver.a",
            &pairs(&[("data_domain", "revenue")]),
            "Rina",
        )
        .await,
    );
    assert!(message.contains("\"revenue\""), "{message}");
    assert!(message.contains("\"data_domain\""), "{message}");
    assert!(message.contains("finance, ops"), "{message}");

    let kept = list_for_asset(&pool, "silver.a").await.expect("list");
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].value, "finance", "a refusal writes nothing");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_undefined_key_a_repeated_key_and_too_many_pairs_are_refused(
    pool: PgPool,
) -> sqlx::Result<()> {
    domain_key(&pool).await;
    let unknown = refusal(set_asset_tags(&pool, "a", &pairs(&[("nope", "x")]), "Rina").await);
    assert!(unknown.contains("\"nope\""), "{unknown}");

    let twice = refusal(
        set_asset_tags(
            &pool,
            "a",
            &pairs(&[("data_domain", "ops"), ("data_domain", "finance")]),
            "Rina",
        )
        .await,
    );
    assert!(twice.contains("given twice"), "{twice}");

    let many: Vec<(String, String)> = (0..21).map(|i| (format!("k{i}"), "v".to_owned())).collect();
    let too_many = refusal(set_asset_tags(&pool, "a", &many, "Rina").await);
    assert!(too_many.contains("at most 20"), "{too_many}");
    assert!(list_for_asset(&pool, "a").await.expect("list").is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn twenty_pairs_are_accepted(pool: PgPool) -> sqlx::Result<()> {
    let mut tags = Vec::new();
    for i in 0..20 {
        let key = format!("k{i}");
        create_key(&pool, &key, &words(&["v"]), "Rina")
            .await
            .expect("key");
        tags.push((key, "v".to_owned()));
    }
    set_asset_tags(&pool, "a", &tags, "Rina")
        .await
        .expect("exactly 20 pairs");
    assert_eq!(list_for_asset(&pool, "a").await.expect("list").len(), 20);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_value_a_table_still_holds_cannot_be_removed_from_its_key(
    pool: PgPool,
) -> sqlx::Result<()> {
    domain_key(&pool).await;
    for asset in ["silver.a", "silver.b"] {
        set_asset_tags(&pool, asset, &pairs(&[("data_domain", "ops")]), "Rina")
            .await
            .expect("tag");
    }

    let message = refusal(update_key_values(&pool, "data_domain", &words(&["finance"])).await);
    assert!(message.contains("\"ops\" (2)"), "{message}");
    let unchanged = list_keys(&pool).await.expect("list");
    assert_eq!(unchanged[0].allowed_values, words(&["finance", "ops"]));

    let widened = update_key_values(&pool, "data_domain", &words(&["finance", "ops", "hr"]))
        .await
        .expect("adding a value is fine");
    assert_eq!(widened.allowed_values, words(&["finance", "ops", "hr"]));

    let unused_removed = update_key_values(&pool, "data_domain", &words(&["ops", "hr"]))
        .await
        .expect("removing an unused value is fine");
    assert_eq!(unused_removed.allowed_values, words(&["ops", "hr"]));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn updating_or_deleting_a_missing_key_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let update = update_key_values(&pool, "nope", &words(&["a"])).await;
    assert!(matches!(update, Err(StoreError::NotFound)), "{update:?}");
    let delete = delete_key(&pool, "nope").await;
    assert!(matches!(delete, Err(StoreError::NotFound)), "{delete:?}");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_key_in_use_cannot_be_deleted_and_the_count_is_in_the_sentence(
    pool: PgPool,
) -> sqlx::Result<()> {
    domain_key(&pool).await;
    for asset in ["silver.a", "silver.b", "serving.c"] {
        set_asset_tags(&pool, asset, &pairs(&[("data_domain", "ops")]), "Rina")
            .await
            .expect("tag");
    }
    let message = refusal(delete_key(&pool, "data_domain").await);
    assert!(message.contains("3 tables"), "{message}");
    assert_eq!(list_keys(&pool).await.expect("list").len(), 1);

    for asset in ["silver.a", "silver.b", "serving.c"] {
        set_asset_tags(&pool, asset, &[], "Rina")
            .await
            .expect("clear");
    }
    delete_key(&pool, "data_domain")
        .await
        .expect("an unused key can go");
    assert!(list_keys(&pool).await.expect("list").is_empty());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_batch_read_returns_each_requested_table_and_no_other(
    pool: PgPool,
) -> sqlx::Result<()> {
    domain_key(&pool).await;
    for (asset, value) in [
        ("silver.a", "ops"),
        ("silver.b", "finance"),
        ("silver.c", "ops"),
    ] {
        set_asset_tags(&pool, asset, &pairs(&[("data_domain", value)]), "Rina")
            .await
            .expect("tag");
    }
    let rows = list_for_assets(&pool, &words(&["silver.a", "silver.c"]))
        .await
        .expect("batch");
    let ids: Vec<_> = rows.iter().map(|r| r.asset_id.as_str()).collect();
    assert_eq!(ids, vec!["silver.a", "silver.c"]);
    assert!(list_for_assets(&pool, &[]).await.expect("empty").is_empty());
    Ok(())
}
