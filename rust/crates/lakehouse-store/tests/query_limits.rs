//! Integration tests for `lakehouse_store::query_limits` against a real
//! Postgres (`#[sqlx::test]`, see `tests/maintenance_policy.rs`).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::query_limits::{
    QueryLimits, ReplacedLimits, RoleLimit, get, role_limits, save,
};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use uuid::Uuid;

/// The migration set this repository applies, compiled in (`sqlx::migrate!`
/// embeds the files at compile time, so a migration edit needs a rebuild
/// before a test result means anything).
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The version of the migration that moves the Query Studio row default
/// from 2,000 to 64,000 (`QS-2`).
const STUDIO_ROWS_MIGRATION: i64 = 116;

async fn role_id(pool: &PgPool, name: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM role WHERE name = $1")
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("seeded role")
}

/// The Query Studio row limit the install row carries.
async fn studio_rows(pool: &PgPool) -> sqlx::Result<i32> {
    sqlx::query_scalar("SELECT studio_rows FROM query_limits WHERE singleton")
        .fetch_one(pool)
        .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_migration_seeds_the_feature_defaults_and_the_struct_default_matches(
    pool: PgPool,
) -> sqlx::Result<()> {
    let stored = get(&pool).await.expect("get");
    assert_eq!(stored, QueryLimits::default());
    assert_eq!(
        stored,
        QueryLimits {
            timeout_seconds: 60,
            chart_rows: 2000,
            map_points: 5000,
            studio_rows: 64000
        }
    );
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}

/// `QS-2` decision 1: the Query Studio row default moves from 2,000 to
/// 64,000, so the row migration `0101` seeded with the old default reads
/// 64,000 once the migration that moves it has run.
#[sqlx::test(migrations = false)]
async fn the_old_query_studio_default_of_two_thousand_moves_to_sixty_four_thousand(
    pool: PgPool,
) -> sqlx::Result<()> {
    // The database as an install had it before the migration: every
    // migration up to the one before it.
    MIGRATOR.run_to(STUDIO_ROWS_MIGRATION - 1, &pool).await?;
    assert_eq!(studio_rows(&pool).await?, 2000, "the old default");

    MIGRATOR.run_to(STUDIO_ROWS_MIGRATION, &pool).await?;

    assert_eq!(studio_rows(&pool).await?, 64000);
    Ok(())
}

/// `QS-2` decision 1: the migration moves the old default and nothing else.
/// A limit an administrator chose stays as it is, and a row written without
/// a value gets the new default.
#[sqlx::test(migrations = false)]
async fn an_administrators_own_query_studio_limit_is_left_alone_by_the_default_migration(
    pool: PgPool,
) -> sqlx::Result<()> {
    MIGRATOR.run_to(STUDIO_ROWS_MIGRATION - 1, &pool).await?;
    sqlx::query("UPDATE query_limits SET studio_rows = 5000 WHERE singleton")
        .execute(&pool)
        .await?;

    MIGRATOR.run_to(STUDIO_ROWS_MIGRATION, &pool).await?;

    assert_eq!(
        studio_rows(&pool).await?,
        5000,
        "a limit an administrator chose is not the old default and stays"
    );
    // A row written without a value gets the column's default, which is
    // the number the migration sets.
    sqlx::query("DELETE FROM query_limits WHERE singleton")
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO query_limits (singleton) VALUES (TRUE)")
        .execute(&pool)
        .await?;
    assert_eq!(studio_rows(&pool).await?, 64000);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_save_round_trips_and_replaces_the_role_set(pool: PgPool) -> sqlx::Result<()> {
    let analyst = role_id(&pool, "Platform Admin").await;
    let install = QueryLimits {
        timeout_seconds: 90,
        chart_rows: 500,
        map_points: 1000,
        studio_rows: 7000,
    };
    let replaced = save(&pool, &install, &[(analyst, 300)], "user-a")
        .await
        .expect("save");
    assert_eq!(
        replaced,
        ReplacedLimits {
            install: Some(QueryLimits::default()),
            roles: Vec::new(),
        },
        "the first save replaced the seeded row and no role limits"
    );
    assert_eq!(get(&pool).await.expect("get"), install);
    let roles = role_limits(&pool).await.expect("roles");
    assert_eq!(roles.len(), 1);
    assert_eq!(roles[0].role_name, "Platform Admin");
    assert_eq!(roles[0].timeout_seconds, 300);

    save(&pool, &install, &[], "user-a").await.expect("save");
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}

/// Plan D3b: a save reports the values it replaced — the install limits and
/// the role limits with their names — so the route can name what changed
/// without reading before it saved.
#[sqlx::test(migrations = "../../migrations")]
async fn a_save_reports_the_install_limits_and_the_role_limits_it_replaced_with_their_names(
    pool: PgPool,
) -> sqlx::Result<()> {
    let analyst = role_id(&pool, "Platform Admin").await;
    let first = QueryLimits {
        timeout_seconds: 90,
        chart_rows: 500,
        map_points: 1000,
        studio_rows: 7000,
    };
    save(&pool, &first, &[(analyst, 300)], "user-a")
        .await
        .expect("first save");

    let second = QueryLimits {
        timeout_seconds: 120,
        chart_rows: 600,
        map_points: 1100,
        studio_rows: 7100,
    };
    let replaced = save(&pool, &second, &[], "user-b")
        .await
        .expect("second save");
    assert_eq!(
        replaced.install,
        Some(first),
        "the install limits the second save replaced"
    );
    assert_eq!(
        replaced.roles,
        vec![RoleLimit {
            role_id: analyst,
            role_name: "Platform Admin".to_owned(),
            timeout_seconds: 300,
        }],
        "the role limits the second save replaced, with the role's name"
    );
    assert_eq!(get(&pool).await.expect("get"), second);
    Ok(())
}

/// Plan D3b: two saves racing each other must serialise, so the second one
/// reports the values the first committed as the ones it replaced, and never
/// the values it would have read before the first committed. Connection A
/// holds an uncommitted update of the install row; `save` on the pool waits
/// for it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_save_waits_for_an_uncommitted_save_and_reports_the_values_it_committed_as_replaced(
    pool: PgPool,
) -> sqlx::Result<()> {
    // Another request's save, not committed yet: the install row is locked
    // until this transaction ends. The transaction holds one connection of
    // the pool, and the save below takes another.
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE query_limits SET chart_rows = 777 WHERE singleton")
        .execute(&mut *tx)
        .await?;

    let new = QueryLimits {
        timeout_seconds: 90,
        chart_rows: 888,
        map_points: 1000,
        studio_rows: 7000,
    };
    let task_pool = pool.clone();
    let save_task = tokio::spawn(async move {
        save(&task_pool, &new, &[], "user-b")
            .await
            .expect("the save that waits")
    });
    // Without the lock the save reads past this uncommitted update at once;
    // with it, the save is still waiting here.
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        !save_task.is_finished(),
        "the save must wait for the uncommitted save before it"
    );

    tx.commit().await?;
    let replaced = save_task.await.expect("the save task joins");
    assert_eq!(
        replaced.install.map(|limits| limits.chart_rows),
        Some(777),
        "the committed update is what the save reports as replaced"
    );
    assert_eq!(
        get(&pool).await.expect("get"),
        new,
        "the save wrote its own values"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_value_outside_the_range_is_refused_and_leaves_the_old_values(
    pool: PgPool,
) -> sqlx::Result<()> {
    let bad = QueryLimits {
        timeout_seconds: 5,
        ..QueryLimits::default()
    };
    assert!(save(&pool, &bad, &[], "u").await.is_err());
    assert_eq!(get(&pool).await.expect("get"), QueryLimits::default());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_role_that_does_not_exist_is_refused_and_nothing_is_saved(
    pool: PgPool,
) -> sqlx::Result<()> {
    let changed = QueryLimits {
        chart_rows: 300,
        ..QueryLimits::default()
    };
    let err = save(&pool, &changed, &[(Uuid::new_v4(), 60)], "u")
        .await
        .expect_err("unknown role");
    assert!(matches!(err, StoreError::ForeignKeyViolation));
    assert_eq!(get(&pool).await.expect("get"), QueryLimits::default());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_role_deletes_its_limit(pool: PgPool) -> sqlx::Result<()> {
    let r: Uuid = sqlx::query_scalar(
        "INSERT INTO role (name, permissions) VALUES ('Temp limit role', '') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    save(&pool, &QueryLimits::default(), &[(r, 120)], "u")
        .await
        .expect("save");
    sqlx::query("DELETE FROM role WHERE id = $1")
        .bind(r)
        .execute(&pool)
        .await?;
    assert_eq!(role_limits(&pool).await.expect("roles"), Vec::new());
    Ok(())
}
