//! Integration tests for `lakehouse_store::annotation` against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database. The Postgres *server*
//! itself is started once per test binary by the `lakehouse-test-support`
//! dev-dependency, which spins up a `testcontainers`-managed Postgres and
//! points `DATABASE_URL` at it before any test runs — no manual
//! `docker compose up`, no external database required. Docker must be
//! reachable from the environment running `cargo test`.
//!
//! Test ids use the two real shapes `routes/catalog.rs` actually emits
//! (see `0032_asset_annotation.sql`'s why-header): a bare, slug-shaped
//! Bronze id (`"commerce_orders"`, no layer prefix) and a `silver.<name>`
//! id (`"silver.mart_orders"`) — never the `"bronze.orders"` shape the
//! first draft of this migration assumed, which no Bronze route ever
//! produces.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::annotation::{
    AnnotationInput, CertificationInput, clear_certification, get_annotation, set_certification,
    upsert_annotation,
};
use sqlx::PgPool;

fn bronze_slug_input() -> AnnotationInput {
    AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: Some("data-eng".to_owned()),
        steward: None,
        tags: vec!["pii".to_owned()],
        description: Some("Order events".to_owned()),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_then_get_round_trips_for_a_bronze_slug_id(pool: PgPool) -> sqlx::Result<()> {
    let input = bronze_slug_input();
    upsert_annotation(&pool, &input).await.expect("upsert");
    let got = get_annotation(&pool, "commerce_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.asset_id, "commerce_orders");
    assert_eq!(got.owner.as_deref(), Some("data-eng"));
    assert_eq!(got.steward, None);
    assert_eq!(got.tags, vec!["pii".to_owned()]);
    assert_eq!(got.description.as_deref(), Some("Order events"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_then_get_round_trips_for_a_silver_prefixed_id(pool: PgPool) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "silver.mart_orders".to_owned(),
        owner: Some("analytics".to_owned()),
        steward: Some("bayu".to_owned()),
        tags: vec!["curated".to_owned(), "finance".to_owned()],
        description: None,
    };
    upsert_annotation(&pool, &input).await.expect("upsert");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.asset_id, "silver.mart_orders");
    assert_eq!(got.steward.as_deref(), Some("bayu"));
    assert_eq!(got.tags, vec!["curated".to_owned(), "finance".to_owned()]);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn get_returns_none_for_an_unannotated_asset(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        get_annotation(&pool, "bronze_unannotated")
            .await
            .expect("query")
            .is_none()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn upsert_replaces_an_existing_annotation(pool: PgPool) -> sqlx::Result<()> {
    upsert_annotation(&pool, &bronze_slug_input())
        .await
        .expect("first upsert");
    upsert_annotation(
        &pool,
        &AnnotationInput {
            asset_id: "commerce_orders".to_owned(),
            owner: Some("new-owner".to_owned()),
            steward: Some("new-steward".to_owned()),
            tags: vec![],
            description: None,
        },
    )
    .await
    .expect("second upsert");

    let got = get_annotation(&pool, "commerce_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.owner.as_deref(), Some("new-owner"));
    assert_eq!(got.steward.as_deref(), Some("new-steward"));
    assert!(got.tags.is_empty());
    assert_eq!(got.description, None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn asset_id_over_200_chars_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "a".repeat(201),
        owner: None,
        steward: None,
        tags: vec![],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn owner_over_128_chars_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: Some("o".repeat(129)),
        steward: None,
        tags: vec![],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn steward_over_128_chars_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: Some("s".repeat(129)),
        tags: vec![],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn description_over_4000_chars_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: vec![],
        description: Some("d".repeat(4001)),
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn more_than_20_tags_is_rejected_by_the_check_constraint(pool: PgPool) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: (0..21).map(|i| format!("tag-{i}")).collect(),
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_tag_over_64_chars_is_rejected_by_the_check_constraint(pool: PgPool) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: vec!["a".repeat(65)],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_tag_with_an_uppercase_letter_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: vec!["PII".to_owned()],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_tag_starting_with_a_hyphen_is_rejected_by_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: vec!["-pii".to_owned()],
        description: None,
    };
    assert!(upsert_annotation(&pool, &input).await.is_err());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn exactly_20_valid_tags_is_accepted(pool: PgPool) -> sqlx::Result<()> {
    let input = AnnotationInput {
        asset_id: "commerce_orders".to_owned(),
        owner: None,
        steward: None,
        tags: (0..20).map(|i| format!("tag-{i}")).collect(),
        description: None,
    };
    upsert_annotation(&pool, &input).await.expect("upsert");
    Ok(())
}

fn mark(status: &str, note: Option<&str>, replacement: Option<&str>) -> CertificationInput {
    CertificationInput {
        asset_id: "silver.mart_orders".to_owned(),
        status: status.to_owned(),
        note: note.map(str::to_owned),
        replacement_asset_id: replacement.map(str::to_owned),
        certified_by: "Ada Lovelace".to_owned(),
    }
}

// DATA-12 F1: each status is stored and read back with who and when.
#[sqlx::test(migrations = "../../migrations")]
async fn a_certified_mark_round_trips(pool: PgPool) -> sqlx::Result<()> {
    set_certification(&pool, &mark("certified", None, None))
        .await
        .expect("set");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.certification.as_deref(), Some("certified"));
    assert_eq!(got.certified_by.as_deref(), Some("Ada Lovelace"));
    assert!(got.certified_at.is_some());
    assert_eq!(got.certification_note, None);
    assert_eq!(got.replacement_asset_id, None);
    // An asset with no annotation row gets one with empty details.
    assert!(got.tags.is_empty());
    assert_eq!(got.owner, None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deprecated_mark_keeps_its_note_and_replacement(pool: PgPool) -> sqlx::Result<()> {
    set_certification(
        &pool,
        &mark("deprecated", Some("Superseded"), Some("silver.mart_orders_v2")),
    )
    .await
    .expect("set");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.certification.as_deref(), Some("deprecated"));
    assert_eq!(got.certification_note.as_deref(), Some("Superseded"));
    assert_eq!(
        got.replacement_asset_id.as_deref(),
        Some("silver.mart_orders_v2")
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn clearing_removes_all_five_columns_and_keeps_the_details(pool: PgPool) -> sqlx::Result<()> {
    let mut details = bronze_slug_input();
    details.asset_id = "silver.mart_orders".to_owned();
    upsert_annotation(&pool, &details).await.expect("upsert");
    set_certification(&pool, &mark("deprecated", Some("Old"), Some("x")))
        .await
        .expect("set");
    clear_certification(&pool, "silver.mart_orders")
        .await
        .expect("clear");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.certification, None);
    assert_eq!(got.certification_note, None);
    assert_eq!(got.replacement_asset_id, None);
    assert_eq!(got.certified_by, None);
    assert_eq!(got.certified_at, None);
    assert_eq!(got.owner.as_deref(), Some("data-eng"));
    Ok(())
}

// DATA-12 F2: editing the details never changes a mark.
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_annotation_after_a_mark_leaves_the_mark(pool: PgPool) -> sqlx::Result<()> {
    set_certification(&pool, &mark("certified", None, None))
        .await
        .expect("set");
    let mut details = bronze_slug_input();
    details.asset_id = "silver.mart_orders".to_owned();
    upsert_annotation(&pool, &details).await.expect("upsert");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.certification.as_deref(), Some("certified"));
    assert_eq!(got.certified_by.as_deref(), Some("Ada Lovelace"));
    assert_eq!(got.owner.as_deref(), Some("data-eng"));
    Ok(())
}

// DATA-12 F2: setting a mark never changes the details.
#[sqlx::test(migrations = "../../migrations")]
async fn set_certification_after_details_leaves_the_details(pool: PgPool) -> sqlx::Result<()> {
    let mut details = bronze_slug_input();
    details.asset_id = "silver.mart_orders".to_owned();
    upsert_annotation(&pool, &details).await.expect("upsert");
    set_certification(&pool, &mark("certified", None, None))
        .await
        .expect("set");
    let got = get_annotation(&pool, "silver.mart_orders")
        .await
        .expect("query")
        .expect("row present");
    assert_eq!(got.owner.as_deref(), Some("data-eng"));
    assert_eq!(got.tags, vec!["pii".to_owned()]);
    assert_eq!(got.description.as_deref(), Some("Order events"));
    assert_eq!(got.certification.as_deref(), Some("certified"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_note_with_certified_is_refused_by_the_check(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        set_certification(&pool, &mark("certified", Some("why"), None))
            .await
            .is_err()
    );
    assert!(
        set_certification(&pool, &mark("certified", None, Some("silver.x")))
            .await
            .is_err()
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_status_is_refused_by_the_check(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        set_certification(&pool, &mark("trusted", None, None))
            .await
            .is_err()
    );
    Ok(())
}
