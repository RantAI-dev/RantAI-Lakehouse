//! Integration tests for `lakehouse_store::reporting_settings` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the
//! database is provided).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap runs for this test binary.
use lakehouse_test_support as _;

use lakehouse_store::reporting_settings::{ReportingSettings, get, upsert};
use sqlx::PgPool;

/// Settings with the formatting defaults of migration `0085` and the empty
/// content-site list of `0086` and the empty denied-roles list of `0087` and the empty
/// report-domain list of `0088`.
fn settings(zone: &str, week: &str) -> ReportingSettings {
    ReportingSettings {
        time_zone: zone.to_owned(),
        week_start: week.to_owned(),
        number_locale: "en-US".to_owned(),
        company_palette: Vec::new(),
        fonts: SHIPPED.iter().map(|f| (*f).to_owned()).collect(),
        content_sites: Vec::new(),
        download_denied_roles: Vec::new(),
        report_allowed_domains: Vec::new(),
    }
}

const SHIPPED: [&str; 5] = [
    "inter",
    "ibm-plex-sans",
    "source-serif",
    "jetbrains-mono",
    "system",
];

#[sqlx::test(migrations = "../../migrations")]
async fn a_deployment_that_never_saved_has_no_row(pool: PgPool) -> sqlx::Result<()> {
    assert_eq!(get(&pool).await.expect("get"), None);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_save_round_trips_and_a_second_save_replaces_the_one_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let first = upsert(&pool, &settings("Europe/Berlin", "sunday"), "user-a")
        .await
        .expect("upsert");
    assert_eq!(first, settings("Europe/Berlin", "sunday"));
    assert_eq!(get(&pool).await.expect("get"), Some(first));

    upsert(&pool, &settings("Asia/Jakarta", "monday"), "user-b")
        .await
        .expect("upsert again");
    let (rows, who): (i64, String) =
        sqlx::query_as("SELECT count(*), max(updated_by) FROM reporting_settings")
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        rows, 1,
        "the second save replaces the row, it does not add one"
    );
    assert_eq!(who, "user-b");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_third_week_start_and_a_zone_that_is_not_shaped_like_one(
    pool: PgPool,
) -> sqlx::Result<()> {
    for (zone, week) in [
        ("Asia/Jakarta", "friday"),
        ("Asia/Jakarta'; DROP TABLE x; --", "monday"),
        ("", "monday"),
        ("Asia//Jakarta", "monday"),
    ] {
        assert!(
            upsert(&pool, &settings(zone, week), "u").await.is_err(),
            "{zone:?} {week:?} should be refused by the table's CHECKs"
        );
    }
    assert_eq!(get(&pool).await.expect("get"), None, "nothing was stored");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_formatting_settings_round_trip_and_a_row_saved_before_them_keeps_the_defaults(
    pool: PgPool,
) -> sqlx::Result<()> {
    // A row as migration 0070 wrote it: the three new columns take their defaults.
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start) \
         VALUES (TRUE, 'UTC', 'monday')",
    )
    .execute(&pool)
    .await?;
    assert_eq!(
        get(&pool).await.expect("get"),
        Some(settings("UTC", "monday"))
    );

    let mut chosen = settings("UTC", "monday");
    chosen.number_locale = "id-ID".to_owned();
    chosen.company_palette = vec!["#1a2b3c".to_owned(), "#FFAA00".to_owned()];
    chosen.fonts = vec!["inter".to_owned(), "system".to_owned()];
    chosen.content_sites = vec![
        "https://img.example".to_owned(),
        "https://cdn.example:8443".to_owned(),
    ];
    chosen.download_denied_roles = vec!["Viewer".to_owned(), "Guest".to_owned()];
    chosen.report_allowed_domains = vec!["example.com".to_owned(), "partner.example".to_owned()];
    assert_eq!(upsert(&pool, &chosen, "u").await.expect("upsert"), chosen);
    assert_eq!(get(&pool).await.expect("get"), Some(chosen));
    Ok(())
}

/// `BI-1` T1 (D7): the table is the backstop for the denied-role list's shape.
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_an_empty_role_name_and_more_than_a_hundred(
    pool: PgPool,
) -> sqlx::Result<()> {
    for roles in [
        vec![String::new()],
        (0..101).map(|i| format!("role-{i}")).collect::<Vec<_>>(),
    ] {
        let mut s = settings("UTC", "monday");
        s.download_denied_roles = roles;
        assert!(upsert(&pool, &s, "u").await.is_err());
    }
    assert_eq!(get(&pool).await.expect("get"), None, "nothing was stored");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_a_third_locale_a_bad_palette_and_a_font_it_does_not_ship(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut cases = Vec::new();
    for locale in ["fr-FR", ""] {
        let mut s = settings("UTC", "monday");
        s.number_locale = locale.to_owned();
        cases.push(s);
    }
    for palette in [
        vec!["#123456".to_owned()],
        vec!["#123456".to_owned(), "red".to_owned()],
        vec!["#123456".to_owned(), "#12345;".to_owned()],
        (0..13).map(|i| format!("#00000{}", i % 10)).collect(),
    ] {
        let mut s = settings("UTC", "monday");
        s.company_palette = palette;
        cases.push(s);
    }
    let mut s = settings("UTC", "monday");
    s.fonts = vec!["comic-sans".to_owned()];
    cases.push(s);
    for case in cases {
        assert!(
            upsert(&pool, &case, "u").await.is_err(),
            "{case:?} should be refused by the table's CHECKs"
        );
    }
    assert_eq!(get(&pool).await.expect("get"), None, "nothing was stored");
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_content_sites_that_are_not_https_origins_or_are_too_many(
    pool: PgPool,
) -> sqlx::Result<()> {
    let mut cases = Vec::new();
    for sites in [
        vec!["http://img.example".to_owned()],
        vec!["https://img.example/path".to_owned()],
        vec!["https://*.example".to_owned()],
        vec![
            "https://a.example".to_owned(),
            "javascript:alert(1)".to_owned(),
        ],
        vec!["https://a.example'; DROP TABLE x; --".to_owned()],
        (0..51).map(|i| format!("https://s{i}.example")).collect(),
    ] {
        let mut s = settings("UTC", "monday");
        s.content_sites = sites;
        cases.push(s);
    }
    for case in cases {
        assert!(
            upsert(&pool, &case, "u").await.is_err(),
            "{case:?} should be refused by the table's CHECKs"
        );
    }
    let mut fifty = settings("UTC", "monday");
    fifty.content_sites = (0..50).map(|i| format!("https://s{i}.example")).collect();
    assert_eq!(upsert(&pool, &fifty, "u").await.expect("fifty"), fifty);
    Ok(())
}

/// `RPT-1` T1 (D1): the table is the backstop for the report-domain list's shape.
#[sqlx::test(migrations = "../../migrations")]
async fn the_table_refuses_an_empty_report_domain_and_more_than_fifty(
    pool: PgPool,
) -> sqlx::Result<()> {
    for domains in [
        vec![String::new()],
        (0..51).map(|i| format!("d{i}.example")).collect::<Vec<_>>(),
    ] {
        let mut s = settings("UTC", "monday");
        s.report_allowed_domains = domains;
        assert!(upsert(&pool, &s, "u").await.is_err());
    }
    assert_eq!(get(&pool).await.expect("get"), None, "nothing was stored");
    Ok(())
}
