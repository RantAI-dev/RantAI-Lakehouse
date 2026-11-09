//! Integration tests for `lakehouse_store::queries` against a real
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

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::audit::{NewAuditEvent, insert as insert_audit_event};
use lakehouse_store::queries::{
    FolderMove, HistoryFilter, RecordHistoryInput, SavedQueryPatch, SavedQueryPlacement,
    SavedQueryScope, SavedQueryViewer, VERSION_KEEP, count_history, create_saved_query,
    create_saved_query_at, create_saved_query_with_parameters, get_history_item, get_saved,
    history_mentioning, history_recent, list_history, list_saved, list_saved_matching,
    list_saved_trash_entries, list_versions, purge_trashed_before, record_history,
    restore_saved_query, revert_to_version, trash_saved_query, update_saved_query,
};
use lakehouse_store::saved_query_access::{
    FolderError, Level, NewGrant, create_folder, delete_folder, ensure_personal_space,
    grantee_labels, grants_for_viewer, manages_folder_and_contents, move_folder_for_viewer,
    rename_folder, replace_grants, rules_for_item, search_grantees,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use sqlx::migrate::Migrator;
use std::borrow::Cow;
use time::OffsetDateTime;
use uuid::Uuid;

/// The migration set this repository applies, compiled in (`sqlx::migrate!`
/// embeds the files at compile time, so a migration edit needs a rebuild
/// before a test result means anything).
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// The seed lands two saved queries, pointed at marts this lakehouse
/// actually serves (`0027_query_ownership.sql` repointed them — the
/// original fixtures named tables that do not exist here, so the only two
/// examples a new user could open both failed on Run).
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_saved_queries(pool: PgPool) -> sqlx::Result<()> {
    let saved = list_saved(
        &pool,
        SavedQueryScope::All {
            viewer: SavedQueryViewer::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(saved.len(), 2);
    assert!(
        saved
            .iter()
            .any(|q| q.title == "Top destinations by visitors")
    );
    assert!(saved.iter().all(|q| q.sql.contains("serving.mart_")));
    Ok(())
}

/// `record_history` writes a row `list_history` then returns, most recent
/// first — the round trip `routes::query::run` depends on.
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_round_trips_through_list(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    assert_eq!(
        list_history(&pool, owner, &HistoryFilter::default())
            .await
            .unwrap(),
        [] as [lakehouse_store::queries::QueryHistoryItem; 0]
    );

    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-1",
            sql: "SELECT 1",
            user: "Bootstrap Admin",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 42,
            scanned_bytes: 1024,
            cost_units: 0.5,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, "q-1");
    assert_eq!(history[0].sql, "SELECT 1");
    assert_eq!(history[0].status, "completed");
    assert!(history[0].at.ends_with('Z'));

    // Another user's history is not this user's: the rows are private to
    // whoever ran them.
    assert_eq!(
        list_history(&pool, Uuid::new_v4(), &HistoryFilter::default())
            .await
            .unwrap(),
        [] as [lakehouse_store::queries::QueryHistoryItem; 0]
    );
    Ok(())
}

/// Recording the same id twice must not fail or duplicate the row — a
/// caller retrying, or a genuinely reused id, degrades to a no-op rather
/// than a 500 that would (per `routes::query::run`'s contract) still not
/// surface to the caller, but would spam logs unnecessarily.
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_is_idempotent_per_id(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    let input = RecordHistoryInput {
        id: "q-dup",
        sql: "SELECT 1",
        user: "Bootstrap Admin",
        owner_id: Some(owner),
        status: "completed",
        duration_ms: 1,
        scanned_bytes: 1,
        cost_units: 0.1,
        workload_class: "hot-analytics",
        engine: "hot-store",
        cache_assisted: false,
        tables_read: &[],
        message: None,
    };
    record_history(&pool, &input).await.unwrap();
    record_history(&pool, &input).await.unwrap();

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    Ok(())
}

/// A `NewAuditEvent` fixture pinned to one query-history resource, mirroring
/// the shape `routes::query::run` would eventually write once WS5 records
/// real query-run audit events.
fn query_history_audit_event(resource_id: &str, action: &str) -> NewAuditEvent {
    NewAuditEvent {
        action: action.to_owned(),
        resource_kind: Some("query_history".to_owned()),
        resource_id: Some(resource_id.to_owned()),
        outcome: "executed".to_owned(),
        ..NewAuditEvent::default()
    }
}

/// WS1 task 1.14 (judge finding J12): no producer mints a synthetic
/// `aud-query-<id>` string anymore — `auditEventId` must be absent until a
/// real `audit_event` row exists for the query-history resource.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_has_no_audit_id_without_a_real_event(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-no-audit",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].audit_event_id, None);
    Ok(())
}

/// Once a real `audit_event` row names the history row's resource, the
/// list resolves the id through the lateral join — the "View audit" link
/// this task is fixing.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_resolves_the_real_audit_event_id(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-audit",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();

    let event = insert_audit_event(&pool, query_history_audit_event("q-audit", "query_sql"))
        .await
        .unwrap();

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(
        history[0].audit_event_id.as_deref(),
        Some(event.id.as_str())
    );
    Ok(())
}

/// Insert an `audit_event` row directly, with an explicit `at`, bypassing
/// `audit::insert`'s `now()` default. Test-only fixture helper: it exists
/// so [`history_item_uses_the_newest_of_several_events_without_duplicating_the_row`]
/// can assert "newest wins" deterministically instead of relying on two
/// `now()`-stamped inserts landing in different microseconds (they are not
/// guaranteed to — see `tests/audit.rs`'s `list_orders_newest_first`, which
/// documents the same tie risk for `audit::insert`).
async fn insert_audit_event_at(pool: &PgPool, id: &str, resource_id: &str, at: OffsetDateTime) {
    sqlx::query(
        "INSERT INTO audit_event (id, at, action, resource_kind, resource_id, outcome) \
         VALUES ($1, $2, 'query_sql', 'query_history', $3, 'executed')",
    )
    .bind(id)
    .bind(at)
    .bind(resource_id)
    .execute(pool)
    .await
    .unwrap();
}

/// Several `audit_event` rows can exist for one query-history resource
/// (e.g. a re-run of the audit pipeline). The lateral join must pick the
/// newest one and must never duplicate the history row — a bare `LEFT
/// JOIN` would return one row per matching event.
#[sqlx::test(migrations = "../../migrations")]
async fn history_item_uses_the_newest_of_several_events_without_duplicating_the_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-multi",
            sql: "SELECT 1",
            user: "anonymous",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "hot-store",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();

    let now = OffsetDateTime::now_utc();
    insert_audit_event_at(&pool, "audit-multi-older", "q-multi", now).await;
    insert_audit_event_at(
        &pool,
        "audit-multi-newer",
        "q-multi",
        now + time::Duration::seconds(60),
    )
    .await;

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1, "the lateral join must not duplicate rows");
    assert_eq!(
        history[0].audit_event_id.as_deref(),
        Some("audit-multi-newer")
    );
    Ok(())
}

/// WS2 §4 — `record_history` carries the real engine and a display-name
/// `user`; ownership scoping lives in the separate `owner_id` column
/// (`0046_query_ownership.sql`), which is what `list_history`'s `owner_id`
/// argument filters on — rewritten from the pre-0046 version of this test,
/// which asserted `user` itself held the principal's uuid (superseded once
/// `owner_id` became its own column).
#[sqlx::test(migrations = "../../migrations")]
async fn record_history_carries_the_real_engine_and_is_scoped_by_owner_id(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-real-user",
            sql: "SELECT 1",
            user: "Real User",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "trino",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();

    let history = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].user, "Real User");
    assert_eq!(history[0].engine, "trino");
    Ok(())
}

/// [`get_history_item`] fetches the same row [`list_history`] would, by
/// id, including the lateral `audit_event` resolution.
#[sqlx::test(migrations = "../../migrations")]
async fn get_history_item_fetches_one_row_by_id(pool: PgPool) -> sqlx::Result<()> {
    record_history(
        &pool,
        &RecordHistoryInput {
            id: "q-get-one",
            sql: "SELECT 1",
            user: "8f14e45f-ceea-467e-bd0d-8fbcae0b0000",
            owner_id: Some(Uuid::new_v4()),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "clickhouse",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();
    let event = insert_audit_event(&pool, query_history_audit_event("q-get-one", "query_sql"))
        .await
        .unwrap();

    let item = get_history_item(&pool, "q-get-one")
        .await
        .unwrap()
        .expect("row must exist");
    assert_eq!(item.id, "q-get-one");
    assert_eq!(item.user, "8f14e45f-ceea-467e-bd0d-8fbcae0b0000");
    assert_eq!(item.engine, "clickhouse");
    assert_eq!(item.audit_event_id.as_deref(), Some(event.id.as_str()));
    Ok(())
}

/// A non-existent id is `Ok(None)`, not an error — the caller (a later
/// task's ownership check) distinguishes "no such query" from a database
/// failure.
#[sqlx::test(migrations = "../../migrations")]
async fn get_history_item_returns_none_for_an_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let item = get_history_item(&pool, "q-does-not-exist").await.unwrap();
    assert_eq!(item, None);
    Ok(())
}

/// `history_mentioning` finds every recent query whose text names the
/// table, whoever ran it — and treats `_` in a table name as a literal,
/// not as the "any one character" wildcard it is in `LIKE`.
#[sqlx::test(migrations = "../../migrations")]
async fn history_mentioning_finds_queries_by_substring_across_users(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (rina, budi) = (Uuid::new_v4(), Uuid::new_v4());
    let run = |id: &'static str, sql: &'static str, owner: Uuid, status: &'static str| {
        let pool = pool.clone();
        async move {
            record_history(
                &pool,
                &RecordHistoryInput {
                    id,
                    sql,
                    user: "someone",
                    owner_id: Some(owner),
                    status,
                    duration_ms: 100,
                    scanned_bytes: 1,
                    cost_units: 0.1,
                    workload_class: "hot-analytics",
                    engine: "hot-store",
                    cache_assisted: false,
                    tables_read: &[],
                    message: None,
                },
            )
            .await
            .unwrap();
        }
    };
    run(
        "q-a",
        "SELECT * FROM silver.orders_clean",
        rina,
        "completed",
    )
    .await;
    run(
        "q-b",
        "select count() from SILVER.ORDERS_CLEAN",
        budi,
        "failed",
    )
    .await;
    run(
        "q-c",
        "SELECT * FROM silver.ordersXclean",
        budi,
        "completed",
    )
    .await;
    run("q-d", "SELECT 1", rina, "completed").await;

    let found = history_mentioning(&pool, 7, &["orders_clean".to_owned()])
        .await
        .unwrap();
    let mut ids: Vec<&str> = found.iter().map(|m| m.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["q-a", "q-b"]);
    assert!(found.iter().all(|m| m.at.ends_with('Z')));
    assert!(
        found
            .iter()
            .any(|m| m.owner_id == Some(budi) && m.status == "failed")
    );

    // Nothing to look for is nothing found, not everything.
    assert_eq!(
        history_mentioning(&pool, 7, &[]).await.unwrap(),
        [] as [lakehouse_store::queries::HistoryMention; 0]
    );
    Ok(())
}

/// `history_recent` (`DATA-11` R2) returns the text of the newest rows
/// first, drops rows older than the window, and stops at the limit.
#[sqlx::test(migrations = "../../migrations")]
async fn history_recent_is_newest_first_and_respects_days_and_limit(
    pool: PgPool,
) -> sqlx::Result<()> {
    for (id, sql) in [
        ("q-old", "SELECT 'old'"),
        ("q-1", "SELECT 1"),
        ("q-2", "SELECT 2"),
        ("q-3", "SELECT 3"),
    ] {
        record_history(
            &pool,
            &RecordHistoryInput {
                id,
                sql,
                user: "someone",
                owner_id: None,
                status: "completed",
                duration_ms: 1,
                scanned_bytes: 1,
                cost_units: 0.1,
                workload_class: "hot-analytics",
                engine: "hot-store",
                cache_assisted: false,
                tables_read: &[],
                message: None,
            },
        )
        .await
        .unwrap();
    }
    // `at` defaults to `now()`; place the four rows explicitly.
    for (id, age_hours) in [("q-old", 240_i32), ("q-1", 3), ("q-2", 2), ("q-3", 1)] {
        sqlx::query(
            "UPDATE query_history SET at = now() - make_interval(hours => $1) WHERE id = $2",
        )
        .bind(age_hours)
        .bind(id)
        .execute(&pool)
        .await?;
    }

    let all = history_recent(&pool, 7, 100).await.unwrap();
    assert_eq!(all, vec!["SELECT 3", "SELECT 2", "SELECT 1"]);

    let two = history_recent(&pool, 7, 2).await.unwrap();
    assert_eq!(two, vec!["SELECT 3", "SELECT 2"]);

    let wide = history_recent(&pool, 30, 100).await.unwrap();
    assert_eq!(wide.len(), 4);
    assert_eq!(wide.last().map(String::as_str), Some("SELECT 'old'"));
    Ok(())
}

/// The two tenants the seeded identity migration creates, and the row
/// that belongs to neither of them.
const GROUP_TENANT: &str = "11111111-1111-4111-8111-000000000001";
const RETAIL_TENANT: &str = "11111111-1111-4111-8111-000000000002";

fn tenant(raw: &str) -> Uuid {
    Uuid::parse_str(raw).expect("a seeded tenant id")
}

/// `SEC-20`: a saved query belongs to the tenant that wrote it. One
/// tenant's rows are that tenant's alone; the rows that belong to no
/// tenant (the ones written before the column existed) are read only by
/// the tenant that owns the shared catalog; and a caller in no tenant
/// reads none.
#[sqlx::test(migrations = "../../migrations")]
async fn saved_queries_are_listed_per_tenant(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let retail = tenant(RETAIL_TENANT);

    // The two writers act as their own tenant and own nobody's query:
    // a viewer with no id has no personal space, so the rows land in the
    // shared space of their tenant.
    let group_writer = viewer(None, Some(group));
    let retail_writer = viewer(None, Some(retail));

    let groups = create_saved_query(
        &pool,
        &group_writer,
        "Group query",
        "SELECT 1",
        "Rina",
        &[],
        None,
    )
    .await
    .unwrap();
    let retails = create_saved_query(
        &pool,
        &retail_writer,
        "Retail query",
        "SELECT 2",
        "Sari",
        &[],
        None,
    )
    .await
    .unwrap();

    // Every row: the two seeded ones (no tenant), and the two written
    // here.
    let all = list_saved(
        &pool,
        SavedQueryScope::All {
            viewer: SavedQueryViewer::default(),
        },
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 4);

    // One tenant's rows, and only that tenant's.
    let own = list_saved(
        &pool,
        SavedQueryScope::Tenant {
            tenant_id: group,
            include_tenantless: false,
            viewer: group_writer.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].id, groups.id);

    let own = list_saved(
        &pool,
        SavedQueryScope::Tenant {
            tenant_id: retail,
            include_tenantless: false,
            viewer: retail_writer.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].id, retails.id);

    // The same tenant, reading the rows that belong to no tenant as
    // well: the seeded ones.
    let own_and_untagged = list_saved(
        &pool,
        SavedQueryScope::Tenant {
            tenant_id: retail,
            include_tenantless: true,
            viewer: retail_writer.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(own_and_untagged.len(), 3);
    assert!(
        own_and_untagged.iter().any(|q| q.id == retails.id),
        "the tenant's own row is in its own list"
    );

    // A caller in no tenant reads no row at all.
    let none = list_saved(&pool, SavedQueryScope::None).await.unwrap();
    assert_eq!(none, [] as [lakehouse_store::queries::SavedQuery; 0]);
    Ok(())
}

// ── The history page's filters (`QS-4`, step 4) ─────────────────────────

/// A listed page's row ids, in the order the page returned them.
fn row_ids(rows: &[lakehouse_store::queries::QueryHistoryItem]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

/// A listed page's row ids, sorted where a test does not care about order.
fn sorted_ids(rows: &[lakehouse_store::queries::QueryHistoryItem]) -> Vec<&str> {
    let mut v = row_ids(rows);
    v.sort_unstable();
    v
}

/// Inserts one history row with the knobs the filter tests turn, and
/// backdates it `age_days` behind `now()` — `record_history` stamps `at`
/// with the column default, so the age is applied afterwards.
#[allow(
    clippy::too_many_arguments,
    reason = "a test fixture: each argument is one knob a filter test turns, and a params struct would bury the one field that differs at a call site"
)]
async fn insert_history_row(
    pool: &PgPool,
    id: &str,
    owner: Uuid,
    status: &str,
    engine: &str,
    duration_ms: i64,
    tables: &[&str],
    message: Option<&str>,
    age_days: i32,
) {
    let tables: Vec<String> = tables.iter().map(|t| (*t).to_owned()).collect();
    record_history(
        pool,
        &RecordHistoryInput {
            id,
            sql: "SELECT 1",
            user: "Bootstrap Admin",
            owner_id: Some(owner),
            status,
            duration_ms,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine,
            cache_assisted: false,
            tables_read: &tables,
            message,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE query_history SET at = now() - make_interval(days => $1) WHERE id = $2")
        .bind(age_days)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

/// Inserts one history row at an exact `at`, so a test can put two rows
/// inside one millisecond — the case the page cursor has to survive.
async fn insert_history_row_at(pool: &PgPool, id: &str, owner: Uuid, at: OffsetDateTime) {
    record_history(
        pool,
        &RecordHistoryInput {
            id,
            sql: "SELECT 1",
            user: "Bootstrap Admin",
            owner_id: Some(owner),
            status: "completed",
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics",
            engine: "clickhouse",
            cache_assisted: false,
            tables_read: &[],
            message: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE query_history SET at = $2 WHERE id = $1")
        .bind(id)
        .bind(at)
        .execute(pool)
        .await
        .unwrap();
}

/// Each filter on its own keeps exactly the rows it names — the one-axis
/// questions the history page's bar asks — and another user's rows answer
/// none of them.
#[sqlx::test(migrations = "../../migrations")]
async fn each_filter_alone_selects_its_own_rows(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    insert_history_row(
        &pool,
        "q-a",
        owner,
        "completed",
        "clickhouse",
        100,
        &["marts.retails"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-b",
        owner,
        "failed",
        "clickhouse",
        5_000,
        &["bronze.orders"],
        Some("Arrow table is missing"),
        3,
    )
    .await;
    insert_history_row(
        &pool,
        "q-c",
        owner,
        "blocked",
        "trino",
        2_500,
        &["marts.retails", "bronze.orders"],
        Some("another tenant claims orders"),
        10,
    )
    .await;

    let failed = list_history(
        &pool,
        owner,
        &HistoryFilter {
            status: Some("failed"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(row_ids(&failed), ["q-b"]);

    let trino = list_history(
        &pool,
        owner,
        &HistoryFilter {
            engine: Some("trino"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(row_ids(&trino), ["q-c"]);

    let slow = list_history(
        &pool,
        owner,
        &HistoryFilter {
            min_duration_ms: Some(1_000),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(row_ids(&slow), ["q-b", "q-c"]);

    let orders = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("orders"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        row_ids(&orders),
        ["q-b", "q-c"],
        "any element of tables_read matches"
    );

    let window = list_history(
        &pool,
        owner,
        &HistoryFilter {
            from: Some(OffsetDateTime::now_utc() - time::Duration::days(5)),
            to: Some(OffsetDateTime::now_utc() - time::Duration::days(2)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        row_ids(&window),
        ["q-b"],
        "the window [now-5d, now-2d] holds exactly the 3-day-old row"
    );

    // Another user's rows never appear, filtered or not.
    let stranger = list_history(
        &pool,
        Uuid::new_v4(),
        &HistoryFilter {
            status: Some("failed"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(row_ids(&stranger), [] as [&str; 0]);
    Ok(())
}

/// The 14-day window is the server's, not the caller's (decision 2): a
/// row 15 days old is never listed, and an older `from` cannot bring it
/// back.
#[sqlx::test(migrations = "../../migrations")]
async fn a_row_fifteen_days_old_is_never_listed_even_when_from_asks_for_it(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    insert_history_row(
        &pool,
        "q-old",
        owner,
        "completed",
        "clickhouse",
        10,
        &["marts.retails"],
        None,
        15,
    )
    .await;
    insert_history_row(
        &pool,
        "q-new",
        owner,
        "completed",
        "clickhouse",
        10,
        &["marts.retails"],
        None,
        0,
    )
    .await;

    let unfiltered = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(
        unfiltered.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        ["q-new"],
        "the default view is the last 14 days"
    );

    let from_thirty_days_back = list_history(
        &pool,
        owner,
        &HistoryFilter {
            from: Some(OffsetDateTime::now_utc() - time::Duration::days(30)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        from_thirty_days_back
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["q-new"],
        "from is clamped to now minus 14 days"
    );
    Ok(())
}

/// Numbered pages walk the rows: 55 rows that share one `at` come back as
/// 50 then 5, with nothing repeated and nothing skipped — the `id`
/// tie-break in the order is what keeps the page boundary stable inside
/// one millisecond. (`QS-4` QA of 2026-11-05 replaced the `before` cursor
/// with `page`; the no-repeat, no-skip assertions stayed.)
#[sqlx::test(migrations = "../../migrations")]
async fn numbered_pages_walk_the_rows_without_repeats_or_skips(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    for i in 0..55 {
        insert_history_row(
            &pool,
            &format!("q-{i:02}"),
            owner,
            "completed",
            "clickhouse",
            1,
            &["marts.retails"],
            None,
            1,
        )
        .await;
    }

    let page_one = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(page_one.len(), 50);

    let page_two = list_history(
        &pool,
        owner,
        &HistoryFilter {
            page: 2,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(page_two.len(), 5);

    let mut seen: Vec<&str> = page_one.iter().map(|r| r.id.as_str()).collect();
    seen.extend(page_two.iter().map(|r| r.id.as_str()));
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seen.len(), 55, "no repeats");
    assert_eq!(sorted.len(), 55, "no skips");
    assert_eq!(sorted[0], "q-00");
    assert_eq!(sorted[54], "q-54");
    Ok(())
}

/// Page 2 of 120 rows is rows 51 to 100, newest first; page 3 is the
/// remaining 20; the count is 120 whichever page is asked for. The rows sit
/// one second apart so the expected ids are a literal order, not something
/// the test recomputes the way the store does.
#[sqlx::test(migrations = "../../migrations")]
async fn page_two_of_120_rows_is_rows_51_to_100_newest_first(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    let base = OffsetDateTime::now_utc() - time::Duration::hours(2);
    for i in 0..120 {
        insert_history_row_at(
            &pool,
            &format!("q-{i:03}"),
            owner,
            base + time::Duration::seconds(i),
        )
        .await;
    }
    let page = |n: i64| HistoryFilter {
        page: n,
        ..Default::default()
    };

    let one = list_history(&pool, owner, &page(1)).await.unwrap();
    let two = list_history(&pool, owner, &page(2)).await.unwrap();
    let three = list_history(&pool, owner, &page(3)).await.unwrap();
    assert_eq!(one.len(), 50);
    assert_eq!(row_ids(&one)[0], "q-119", "page 1 starts at the newest row");
    assert_eq!(row_ids(&one)[49], "q-070");
    assert_eq!(two.len(), 50);
    assert_eq!(row_ids(&two)[0], "q-069", "page 2 is rows 51 to 100");
    assert_eq!(row_ids(&two)[49], "q-020");
    assert_eq!(
        row_ids(&three),
        (0..20)
            .rev()
            .map(|i| format!("q-{i:03}"))
            .collect::<Vec<_>>()
    );

    for n in 1..=3 {
        assert_eq!(
            count_history(&pool, owner, &page(n)).await.unwrap(),
            120,
            "the total does not depend on the page"
        );
    }
    Ok(())
}

/// A page past the end is an empty page, not an error, and the count stays
/// true; so does a page so large that its offset would overflow.
#[sqlx::test(migrations = "../../migrations")]
async fn a_page_past_the_end_is_empty_and_the_count_stays_true(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    for i in 0..3 {
        insert_history_row(
            &pool,
            &format!("q-{i}"),
            owner,
            "completed",
            "clickhouse",
            1,
            &[],
            None,
            0,
        )
        .await;
    }
    for page in [2, 3, 1_000_000, i64::MAX] {
        let filter = HistoryFilter {
            page,
            ..Default::default()
        };
        assert!(
            list_history(&pool, owner, &filter)
                .await
                .unwrap()
                .is_empty(),
            "page {page} is past the end"
        );
        assert_eq!(count_history(&pool, owner, &filter).await.unwrap(), 3);
    }
    Ok(())
}

/// The count carries the same `WHERE` as the list: each filter alone, the
/// 14-day window, and the owner scope, so `total` can never count a row the
/// list would not show.
#[sqlx::test(migrations = "../../migrations")]
async fn the_count_follows_every_filter_the_list_follows(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    insert_history_row(
        &pool,
        "q-a",
        owner,
        "completed",
        "clickhouse",
        100,
        &["marts.retails"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-b",
        owner,
        "failed",
        "clickhouse",
        5_000,
        &["bronze.orders"],
        Some("Arrow table is missing"),
        3,
    )
    .await;
    insert_history_row(
        &pool,
        "q-c",
        owner,
        "blocked",
        "trino",
        2_500,
        &["marts.retails", "bronze.orders"],
        Some("another tenant claims orders"),
        10,
    )
    .await;
    insert_history_row(
        &pool,
        "q-d",
        owner,
        "stopped",
        "clickhouse",
        7_000,
        &[],
        Some("Stopped"),
        1,
    )
    .await;
    // Outside the 14-day window: listed by nobody, counted by nobody.
    insert_history_row(
        &pool,
        "q-old",
        owner,
        "failed",
        "trino",
        9_000,
        &["bronze.orders"],
        None,
        20,
    )
    .await;
    // Another user's row.
    insert_history_row(
        &pool,
        "q-other",
        Uuid::new_v4(),
        "failed",
        "trino",
        9_000,
        &["bronze.orders"],
        None,
        0,
    )
    .await;

    let count = |filter: HistoryFilter<'static>| {
        let pool = pool.clone();
        async move { count_history(&pool, owner, &filter).await.unwrap() }
    };
    let list = |filter: HistoryFilter<'static>| {
        let pool = pool.clone();
        async move {
            let rows = list_history(&pool, owner, &filter).await.unwrap();
            i64::try_from(rows.len()).unwrap()
        }
    };
    let filters: [(&str, HistoryFilter<'static>, i64); 8] = [
        (
            "no filter: the window and the owner only",
            HistoryFilter::default(),
            4,
        ),
        (
            "status",
            HistoryFilter {
                status: Some("failed"),
                ..Default::default()
            },
            1,
        ),
        (
            "status stopped",
            HistoryFilter {
                status: Some("stopped"),
                ..Default::default()
            },
            1,
        ),
        (
            "engine",
            HistoryFilter {
                engine: Some("trino"),
                ..Default::default()
            },
            1,
        ),
        (
            "min duration",
            HistoryFilter {
                min_duration_ms: Some(2_000),
                ..Default::default()
            },
            3,
        ),
        (
            "table",
            HistoryFilter {
                table: Some("orders"),
                ..Default::default()
            },
            2,
        ),
        (
            "from and to",
            HistoryFilter {
                from: Some(OffsetDateTime::now_utc() - time::Duration::days(5)),
                to: Some(OffsetDateTime::now_utc() - time::Duration::days(2)),
                ..Default::default()
            },
            1,
        ),
        (
            "several at once",
            HistoryFilter {
                engine: Some("clickhouse"),
                min_duration_ms: Some(2_000),
                table: Some("orders"),
                ..Default::default()
            },
            1,
        ),
    ];
    for (name, filter, expected) in filters {
        assert_eq!(count(filter.clone()).await, expected, "count: {name}");
        assert_eq!(
            list(filter).await,
            expected,
            "list agrees with count: {name}"
        );
    }

    let stranger = count_history(&pool, Uuid::new_v4(), &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(stranger, 0, "a user with no runs has a total of 0");
    Ok(())
}

/// `%` and `_` in the table text are matched literally (decision 7), while
/// the match itself stays a case-insensitive contains.
#[sqlx::test(migrations = "../../migrations")]
async fn a_percent_or_underscore_in_the_table_filter_is_literal(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    insert_history_row(
        &pool,
        "q-plain",
        owner,
        "completed",
        "clickhouse",
        1,
        &["bronze.orders"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-wild",
        owner,
        "completed",
        "clickhouse",
        1,
        &["marts.ord%ers"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-under",
        owner,
        "completed",
        "clickhouse",
        1,
        &["bronze.o_ders"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-caps",
        owner,
        "completed",
        "clickhouse",
        1,
        &["BRONZE.ORDERS"],
        None,
        0,
    )
    .await;
    insert_history_row(
        &pool,
        "q-suffix",
        owner,
        "completed",
        "clickhouse",
        1,
        &["bronze.orders_old"],
        None,
        0,
    )
    .await;

    let contains = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("orders"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sorted_ids(&contains),
        ["q-caps", "q-plain", "q-suffix"],
        "case-insensitive, any element, contains"
    );

    let literal_percent = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("ord%"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sorted_ids(&literal_percent),
        ["q-wild"],
        "% is a literal percent"
    );

    let literal_underscore = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("o_ders"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sorted_ids(&literal_underscore),
        ["q-under"],
        "_ is a literal underscore"
    );
    Ok(())
}

/// Two rows of one owner inside one millisecond come back exactly once
/// across the page boundary, in the order a row's `at` shows.
///
/// The value a row's `at` leaves the store with is cut to milliseconds
/// (`iso_millis`), so two rows inside one millisecond look equal to the
/// caller and only the id tells them apart. The order therefore cuts `at` to
/// the millisecond and falls back to the id, so the page boundary is
/// stable and matches what the console shows (`QS-4` QA of 2026-11-05: the
/// `before` cursor is gone, this ordering assertion stays).
///
/// The two rows are placed so their ids disagree with their microseconds:
/// `q-a` is the newer of the two by a microsecond and the older by id. An
/// order on the raw microsecond puts `q-a` at the end of the page, which
/// shows two rows of one displayed millisecond out of id order.
#[sqlx::test(migrations = "../../migrations")]
async fn two_rows_in_one_millisecond_come_back_exactly_once_across_the_page_boundary(
    pool: PgPool,
) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    // A whole second an hour ago: inside the window the store clamps to,
    // and with no sub-second part to round away.
    let base = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("a whole second")
        - time::Duration::hours(1);
    // A page holds 50 rows, so 49 newer rows put the two rows of one
    // millisecond at the end of the first page and at the start of the
    // next — the boundary between page 1 and page 2.
    for i in 0..49 {
        insert_history_row_at(
            &pool,
            &format!("q-fill-{i:02}"),
            owner,
            base + time::Duration::seconds(1),
        )
        .await;
    }
    // The newer of the two by microsecond is the older one by id: only an
    // order that cuts to the millisecond and falls back to the id puts `q-b`
    // at the end of the page. An order on the raw microsecond puts `q-a`
    // there.
    insert_history_row_at(
        &pool,
        "q-a",
        owner,
        base + time::Duration::microseconds(150),
    )
    .await;
    insert_history_row_at(
        &pool,
        "q-b",
        owner,
        base + time::Duration::microseconds(100),
    )
    .await;

    let page_one = list_history(&pool, owner, &HistoryFilter::default())
        .await
        .unwrap();
    assert_eq!(page_one.len(), 50);
    let last = page_one.last().unwrap();
    assert!(
        last.at.ends_with('Z'),
        "a row's at is an ISO timestamp: {}",
        last.at
    );

    let page_two = list_history(
        &pool,
        owner,
        &HistoryFilter {
            page: 2,
            ..Default::default()
        },
    )
    .await
    .unwrap();

    // The fixture holds 51 rows: 49 fill rows and the two of one
    // millisecond. Across the boundary each of them comes back exactly
    // once — none repeated, none skipped.
    let mut seen: Vec<&str> = page_one.iter().map(|r| r.id.as_str()).collect();
    seen.extend(page_two.iter().map(|r| r.id.as_str()));
    let mut sorted = seen.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(seen.len(), sorted.len(), "no repeats");
    assert_eq!(seen.len(), 51, "no skips");

    // Which row ended the page, and which one came back with the next page:
    // the id tie-break inside the millisecond, the same order the
    // comparison walks.
    assert_eq!(
        last.id, "q-b",
        "the greater id of the millisecond ends the page"
    );
    assert_eq!(
        row_ids(&page_two),
        ["q-a"],
        "the row inside the last page's millisecond comes back"
    );
    Ok(())
}

/// A `\` in the table text is matched literally: it is the escape
/// character of the match, so it has to survive the chain that makes `%`
/// and `_` literal — and it never turns into a wildcard.
#[sqlx::test(migrations = "../../migrations")]
async fn a_backslash_in_the_table_filter_matches_literally(pool: PgPool) -> sqlx::Result<()> {
    let owner = Uuid::new_v4();
    insert_history_row(
        &pool,
        "q-backslash",
        owner,
        "completed",
        "clickhouse",
        1,
        &["bronze\\orders"],
        None,
        0,
    )
    .await;

    let literal = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("bronze\\orders"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        sorted_ids(&literal),
        ["q-backslash"],
        "a backslash is a literal backslash"
    );

    let wildcard = list_history(
        &pool,
        owner,
        &HistoryFilter {
            table: Some("bronze%orders"),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(
        wildcard.is_empty(),
        "a percent is still a literal percent: {:?}",
        row_ids(&wildcard)
    );
    Ok(())
}

// ── The saved-query spaces, the history and the trash (`QS-3`) ──────────

/// The seeded people, role and query the saved-query tests use.
const RINA: &str = "33333333-3333-4333-8333-000000000001";
const SARI: &str = "33333333-3333-4333-8333-000000000005";
const ANALYST_ROLE: &str = "22222222-2222-4222-8222-000000000001";
const SEEDED_QUERY: &str = "99999999-9999-4999-8999-000000000001";

/// A viewer who is one person of one tenant, holding no role and no
/// `"*:*"`: the plainest reader the saved-query paths see.
fn viewer(id: Option<Uuid>, tenant_id: Option<Uuid>) -> SavedQueryViewer {
    SavedQueryViewer {
        id,
        role_names: Vec::new(),
        tenant_id,
        all: false,
    }
}

/// A viewer who holds `"*:*"` in one tenant: it manages everything of the
/// organisation, so a test that is about the tree's own rules (a cycle, the
/// depth, another tenant's folder) is not also about the access rule.
fn manager_of_everything(tenant_id: Uuid) -> SavedQueryViewer {
    SavedQueryViewer {
        id: None,
        role_names: Vec::new(),
        tenant_id: Some(tenant_id),
        all: true,
    }
}

/// The seeded people, as ids.
fn person(raw: &str) -> Uuid {
    Uuid::parse_str(raw).expect("a seeded person id")
}

/// Every row that existed before the spaces were introduced stays in the
/// shared space and gets a first version labelled as such: the history of
/// an old query starts where the migration found it.
#[sqlx::test(migrations = "../../migrations")]
async fn the_queries_that_existed_before_the_migration_keep_their_place_and_get_a_first_version(
    pool: PgPool,
) -> sqlx::Result<()> {
    let seed_id = Uuid::parse_str(SEEDED_QUERY).expect("a seeded query id");
    let scope = SavedQueryScope::All {
        viewer: SavedQueryViewer::default(),
    };

    let saved = get_saved(&pool, SEEDED_QUERY, &scope)
        .await
        .unwrap()
        .expect("the seeded query is there");
    assert_eq!(
        saved.folder_id, None,
        "a query that existed before the spaces stays in the shared space"
    );
    assert_eq!(
        saved.level, "view",
        "the shared space is open to the organisation"
    );

    let versions = list_versions(&pool, seed_id).await.unwrap();
    assert_eq!(versions.len(), 1, "the old row has exactly one version");
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[0].label, "Saved before history");
    assert_eq!(versions[0].title, saved.title);
    assert_eq!(versions[0].sql, saved.sql);
    Ok(())
}

/// A personal space is one folder per tenant and person: it is created
/// once and found again, never made twice.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_space_is_one_per_person_and_tenant(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = person(RINA);
    let caller = viewer(Some(rina), Some(tenant_id));

    let first = create_saved_query(&pool, &caller, "Mine", "SELECT 1", "Rina Wijaya", &[], None)
        .await
        .unwrap();
    let second = create_saved_query(
        &pool,
        &caller,
        "Mine too",
        "SELECT 2",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();

    assert!(
        first.folder_id.is_some(),
        "a query of a person lands in their personal space"
    );
    assert_eq!(
        first.folder_id, second.folder_id,
        "one personal space per person, not one per query"
    );

    let spaces: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saved_query_folder WHERE is_personal")
            .fetch_one(&pool)
            .await?;
    assert_eq!(spaces, 1, "the personal space is one row");

    let second_space = sqlx::query(
        "INSERT INTO saved_query_folder (tenant_id, name, is_personal, owner_id) \
         VALUES ($1, 'Personal space of Rina Wijaya', true, $2)",
    )
    .bind(tenant_id)
    .bind(rina)
    .execute(&pool)
    .await;
    assert!(
        second_space.is_err(),
        "the database refuses a second personal space for the same person"
    );
    Ok(())
}

/// A folder's name is at most a hundred characters: the tree says so in
/// the database, not only in the route.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_name_is_at_most_a_hundred_characters(pool: PgPool) -> sqlx::Result<()> {
    let too_long = sqlx::query("INSERT INTO saved_query_folder (tenant_id, name) VALUES ($1, $2)")
        .bind(tenant(GROUP_TENANT))
        .bind("x".repeat(101))
        .execute(&pool)
        .await;
    assert!(
        too_long.is_err(),
        "a name of 101 characters is refused by the database"
    );

    sqlx::query("INSERT INTO saved_query_folder (tenant_id, name) VALUES ($1, $2)")
        .bind(tenant(GROUP_TENANT))
        .bind("x".repeat(100))
        .execute(&pool)
        .await?;
    Ok(())
}

/// A version is written for every change of the title or the SQL, and for
/// none of the changes that leave both as they were.
#[sqlx::test(migrations = "../../migrations")]
async fn a_version_is_written_for_a_change_and_skipped_when_nothing_changed(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let saved = create_saved_query(
        &pool,
        &caller,
        "First",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    assert_eq!(
        list_versions(&pool, query_id).await.unwrap().len(),
        1,
        "the create writes the first version"
    );

    let unchanged = update_saved_query(
        &pool,
        &caller,
        &saved.id,
        &SavedQueryPatch {
            title: Some("First".to_owned()),
            sql: Some("SELECT 1".to_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(unchanged.title, "First");
    assert_eq!(
        list_versions(&pool, query_id).await.unwrap().len(),
        1,
        "a change that leaves the title and the SQL as they were writes no version"
    );

    update_saved_query(
        &pool,
        &caller,
        &saved.id,
        &SavedQueryPatch {
            title: Some("Second".to_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), 2, "a new title is a new version");
    assert_eq!(versions[0].version, 2);
    assert_eq!(versions[0].title, "Second");
    Ok(())
}

/// The two definitions the `QS-5` tests save (decision 3): the store keeps them as
/// the JSON the route validated and never reads inside them.
fn region_definitions() -> Value {
    json!([{"name": "region", "type": "text", "default": "EU"}])
}

/// The same query with a different default, so a patch that changes only
/// this differs from the stored list by a single value.
fn region_definitions_with_default_us() -> Value {
    json!([{"name": "region", "type": "text", "default": "US"}])
}

/// `query-parameters.md` decision 3: a saved query keeps its parameters beside its SQL. A change
/// of the parameters alone writes a version, a change that leaves them as
/// they were writes none, a change of the title alone keeps them, and a
/// revert brings back the parameters of the version it went back to.
#[sqlx::test(migrations = "../../migrations")]
async fn a_saved_query_keeps_its_parameters_in_every_version_and_a_revert_restores_them(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let scope = SavedQueryScope::All {
        viewer: caller.clone(),
    };
    let saved = create_saved_query_with_parameters(
        &pool,
        &caller,
        "By region",
        "SELECT {{region}}",
        &region_definitions(),
        "Rina Wijaya",
        &[],
        SavedQueryPlacement::SharedSpace,
    )
    .await
    .unwrap();
    assert_eq!(saved.parameters, region_definitions());
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    let read = get_saved(&pool, &saved.id, &scope).await.unwrap().unwrap();
    assert_eq!(read.parameters, region_definitions(), "get carries them");
    let listed = list_saved(&pool, scope.clone()).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|q| q.id == saved.id)
            .map(|q| q.parameters.clone()),
        Some(region_definitions()),
        "the list carries them"
    );
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].parameters, region_definitions());

    // The same list sent again changes nothing.
    update_saved_query(
        &pool,
        &caller,
        &saved.id,
        &SavedQueryPatch {
            parameters: Some(region_definitions()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        list_versions(&pool, query_id).await.unwrap().len(),
        1,
        "parameters that are as they were write no version"
    );

    // A change of the parameters alone is a version.
    let patched = update_saved_query(
        &pool,
        &caller,
        &saved.id,
        &SavedQueryPatch {
            parameters: Some(region_definitions_with_default_us()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(patched.parameters, region_definitions_with_default_us());
    assert_eq!(patched.sql, "SELECT {{region}}", "the SQL did not move");
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), 2, "a parameters-only change is a version");
    assert_eq!(
        versions[0].parameters,
        region_definitions_with_default_us(),
        "the new version holds the new parameters"
    );
    assert_eq!(
        versions[1].parameters,
        region_definitions(),
        "the old version keeps the old ones"
    );

    // A title-only patch leaves the parameters where they are.
    let renamed = update_saved_query(
        &pool,
        &caller,
        &saved.id,
        &SavedQueryPatch {
            title: Some("By region, renamed".to_owned()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(renamed.parameters, region_definitions_with_default_us());
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(
        versions[0].parameters,
        region_definitions_with_default_us(),
        "a version written for a new title still holds the current parameters"
    );

    // Going back to version 1 brings back its parameters with its SQL.
    let reverted = revert_to_version(&pool, &caller, query_id, 1)
        .await
        .unwrap();
    assert_eq!(reverted.parameters, region_definitions());
    assert_eq!(reverted.title, "By region");
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), 4, "the revert is itself a version");
    assert_eq!(versions[0].label, "Restored version 1");
    assert_eq!(versions[0].parameters, region_definitions());
    Ok(())
}

/// A query saved the way every caller did before `QS-5` has no parameters:
/// the list is empty, not missing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_query_saved_without_parameters_reads_an_empty_list(pool: PgPool) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let saved = create_saved_query(
        &pool,
        &caller,
        "Plain",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    assert_eq!(saved.parameters, json!([]));
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions[0].parameters, json!([]));
    Ok(())
}

/// The migrations a database that stopped at `version` would have applied.
fn migrations_up_to(version: i64) -> Migrator {
    let upto: Vec<_> = MIGRATOR
        .iter()
        .filter(|m| m.version <= version)
        .cloned()
        .collect();
    Migrator {
        migrations: Cow::Owned(upto),
        ..Migrator::DEFAULT
    }
}

/// `0131` gives every row written before it an empty parameter list, on the
/// query and on its versions, and the store reads them back as such.
#[sqlx::test(migrations = false)]
async fn migration_0131_gives_old_queries_and_versions_an_empty_parameter_list(
    pool: PgPool,
) -> sqlx::Result<()> {
    migrations_up_to(130).run(&pool).await.unwrap();
    let query_id = Uuid::new_v4();
    sqlx::query("INSERT INTO saved_query (id, title, sql, owner) VALUES ($1, 'Old', 'SELECT 1', 'Rina Wijaya')")
        .bind(query_id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO saved_query_version (query_id, version, title, sql, author_name, label) \
         VALUES ($1, 7, 'Old', 'SELECT 1', 'Rina Wijaya', '')",
    )
    .bind(query_id)
    .execute(&pool)
    .await?;

    MIGRATOR.run(&pool).await.unwrap();

    let on_query: Value = sqlx::query_scalar("SELECT parameters FROM saved_query WHERE id = $1")
        .bind(query_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(on_query, json!([]));
    let on_version: Value =
        sqlx::query_scalar("SELECT parameters FROM saved_query_version WHERE query_id = $1")
            .bind(query_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(on_version, json!([]));
    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions[0].parameters, json!([]));
    Ok(())
}

/// Only the last fifteen versions of a query are kept: the older ones go
/// in the same transaction as the change that pushed them out.
#[sqlx::test(migrations = "../../migrations")]
async fn only_the_last_fifteen_versions_are_kept(pool: PgPool) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let saved = create_saved_query(
        &pool,
        &caller,
        "Versioned",
        "SELECT 0",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");

    for n in 1..=20 {
        update_saved_query(
            &pool,
            &caller,
            &saved.id,
            &SavedQueryPatch {
                sql: Some(format!("SELECT {n}")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    let versions = list_versions(&pool, query_id).await.unwrap();
    assert_eq!(versions.len(), VERSION_KEEP, "the history is capped");
    assert_eq!(
        versions[0].version, 21,
        "the newest version is the last one written"
    );
    assert_eq!(
        versions[VERSION_KEEP - 1].version,
        7,
        "the oldest kept version is fifteen back from the newest"
    );
    Ok(())
}

/// A trashed query leaves the list, waits in the trash and comes back
/// where it was.
#[sqlx::test(migrations = "../../migrations")]
async fn a_trashed_query_leaves_the_list_and_can_be_restored(pool: PgPool) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let scope = SavedQueryScope::All {
        viewer: caller.clone(),
    };
    let saved = create_saved_query(
        &pool,
        &caller,
        "Doomed",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();

    trash_saved_query(&pool, &caller, &saved.id).await.unwrap();
    assert!(
        !list_saved(&pool, scope.clone())
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "a trashed query is not listed"
    );
    assert!(
        list_saved_trash_entries(&pool, &scope)
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "the trash holds it"
    );

    let restored = restore_saved_query(&pool, &caller, &saved.id)
        .await
        .unwrap();
    assert_eq!(restored.id, saved.id);
    assert_eq!(
        restored.folder_id, saved.folder_id,
        "it comes back in its folder"
    );
    assert!(
        list_saved(&pool, scope.clone())
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "it is back in the list"
    );
    assert!(
        !list_saved_trash_entries(&pool, &scope)
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "and out of the trash"
    );
    Ok(())
}

/// The sweep removes the rows that were trashed more than thirty days
/// ago, and leaves the younger ones alone.
#[sqlx::test(migrations = "../../migrations")]
async fn the_sweep_removes_the_rows_trashed_more_than_thirty_days_ago(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller = viewer(Some(person(RINA)), Some(tenant(GROUP_TENANT)));
    let old = create_saved_query(&pool, &caller, "Old", "SELECT 1", "Rina Wijaya", &[], None)
        .await
        .unwrap();
    let fresh = create_saved_query(
        &pool,
        &caller,
        "Fresh",
        "SELECT 2",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    trash_saved_query(&pool, &caller, &old.id).await.unwrap();
    trash_saved_query(&pool, &caller, &fresh.id).await.unwrap();
    sqlx::query("UPDATE saved_query SET deleted_at = now() - interval '31 days' WHERE id = $1")
        .bind(Uuid::parse_str(&old.id).expect("the saved query id"))
        .execute(&pool)
        .await?;

    let removed = purge_trashed_before(&pool, OffsetDateTime::now_utc() - time::Duration::days(30))
        .await
        .unwrap();
    assert_eq!(removed, 1, "the row trashed thirty-one days ago is gone");

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM saved_query WHERE id = $1")
        .bind(Uuid::parse_str(&fresh.id).expect("the saved query id"))
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 1, "the row trashed a moment ago is still there");
    Ok(())
}

/// The search matches the title or the SQL, and the wildcards are
/// literals: a `%` is a percent sign, not "anything".
#[sqlx::test(migrations = "../../migrations")]
async fn the_search_matches_the_title_or_the_sql_and_treats_the_wildcards_literally(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let caller = viewer(Some(person(RINA)), Some(tenant_id));
    let scope = SavedQueryScope::Tenant {
        tenant_id,
        include_tenantless: false,
        viewer: caller.clone(),
    };
    create_saved_query(
        &pool,
        &caller,
        "Revenue by region",
        "SELECT region FROM gold.revenue",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();

    let by_title = list_saved_matching(&pool, &scope, Some("revenue"))
        .await
        .unwrap();
    assert!(
        by_title.iter().any(|q| q.title == "Revenue by region"),
        "the title matches"
    );

    let by_sql = list_saved_matching(&pool, &scope, Some("gold."))
        .await
        .unwrap();
    assert!(
        by_sql.iter().any(|q| q.title == "Revenue by region"),
        "the SQL matches"
    );

    assert!(
        list_saved_matching(&pool, &scope, Some("%"))
            .await
            .unwrap()
            .is_empty(),
        "a percent is a literal percent"
    );
    assert!(
        list_saved_matching(&pool, &scope, Some("_"))
            .await
            .unwrap()
            .is_empty(),
        "an underscore is a literal underscore"
    );
    Ok(())
}

/// A personal space is a wall: another person sees nothing of what is
/// inside it, until a rule opens it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_space_hides_its_queries_from_everybody_else(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let saved = create_saved_query(
        &pool,
        &rina,
        "Private",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let sari_scope = SavedQueryScope::All {
        viewer: sari.clone(),
    };

    assert!(
        !list_saved(&pool, sari_scope.clone())
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "a personal space gives nothing to another person"
    );

    replace_grants(
        &pool,
        Some(Uuid::parse_str(&saved.id).expect("the saved query id")),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::View,
        }],
        None,
    )
    .await
    .unwrap();

    let listed = list_saved(&pool, sari_scope).await.unwrap();
    let opened = listed
        .iter()
        .find(|q| q.id == saved.id)
        .expect("a rule opens the personal space");
    assert_eq!(opened.level, "view", "the rule gives exactly what it names");
    Ok(())
}

/// Another tenant's query gives nothing: it is not listed, and a write
/// against it finds no row at all.
#[sqlx::test(migrations = "../../migrations")]
async fn another_tenants_query_gives_nothing(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let retail = tenant(RETAIL_TENANT);
    let rina = viewer(Some(person(RINA)), Some(group));
    // The query sits in the shared space, so only the tenant check can
    // hide it from Sari (`QS-3` review finding 8).
    let saved = create_saved_query_at(
        &pool,
        &rina,
        "The group's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        SavedQueryPlacement::SharedSpace,
    )
    .await
    .unwrap();

    let sari = viewer(Some(person(SARI)), Some(retail));
    let sari_scope = SavedQueryScope::Tenant {
        tenant_id: retail,
        include_tenantless: false,
        viewer: sari.clone(),
    };
    assert!(
        !list_saved(&pool, sari_scope)
            .await
            .unwrap()
            .iter()
            .any(|q| q.id == saved.id),
        "another tenant's query is not listed"
    );

    let written = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            title: Some("Mine now".to_owned()),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(written, Err(lakehouse_store::StoreError::NotFound)),
        "another tenant's query is not found at all"
    );
    Ok(())
}

// ── A move across a space needs manage (plan 55) ─────────────────────────

/// An edit grantee cannot move a query across a space: the move is
/// refused, and neither the title nor the SQL the same body carries is
/// written.
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_grantees_move_across_a_space_is_refused_and_changes_nothing(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let saved = create_saved_query(
        &pool,
        &rina,
        "Private",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let personal = saved.folder_id.clone().expect("a personal space folder");

    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let refused = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            title: Some("Hijacked".to_owned()),
            sql: Some("SELECT 2".to_owned()),
            folder: Some(FolderMove::Root),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(refused, Err(lakehouse_store::StoreError::NotAllowed)),
        "an edit grantee cannot move a query across a space: {refused:?}"
    );

    let rina_scope = SavedQueryScope::Tenant {
        tenant_id,
        include_tenantless: false,
        viewer: rina,
    };
    let row = get_saved(&pool, &saved.id, &rina_scope)
        .await
        .unwrap()
        .expect("the query is there");
    assert_eq!(
        row.title, "Private",
        "the title of the same body is not written"
    );
    assert_eq!(
        row.sql, "SELECT 1",
        "the SQL of the same body is not written"
    );
    assert_eq!(
        row.folder_id,
        Some(personal),
        "the query stays in the personal space"
    );
    Ok(())
}

/// A manager's move across a space succeeds: a rule that names the mover
/// at `manage` is enough, the author is not the only manager.
#[sqlx::test(migrations = "../../migrations")]
async fn a_managers_move_across_a_space_succeeds(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let saved = create_saved_query(
        &pool,
        &rina,
        "Private",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Manage,
        }],
        None,
    )
    .await
    .unwrap();
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let moved = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            folder: Some(FolderMove::Root),
            ..Default::default()
        },
    )
    .await;
    assert!(
        moved.is_ok(),
        "a manager moves the query across a space: {moved:?}"
    );
    assert_eq!(
        moved.unwrap().folder_id,
        None,
        "the query sits in the shared space"
    );
    Ok(())
}

/// A move inside one space needs manage too: a query of Rina's personal
/// space does not move into a folder of that same space on an edit rule.
/// This test used to expect success: an edit rule let a query move inside one space,
/// and that was the way to publish, hide or raise a query in two steps
/// (`QS-3` review of #382, BLOCKER 1).
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_grantees_move_inside_one_space_is_refused_and_changes_nothing(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let personal = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    let week = create_folder(&pool, tenant_id, None, "Week", Some(personal.id))
        .await
        .unwrap();
    let saved = create_saved_query(
        &pool,
        &rina,
        "Private",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
    )
    .await
    .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let refused = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            title: Some("Hijacked".to_owned()),
            folder: Some(FolderMove::Into(week.id)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(refused, Err(lakehouse_store::StoreError::NotAllowed)),
        "a move inside one space needs manage too: {refused:?}"
    );
    let (title, folder): (String, Option<Uuid>) =
        sqlx::query_as("SELECT title, folder_id FROM saved_query WHERE id = $1")
            .bind(query_id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        title, "Private",
        "the title of the same body is not written"
    );
    assert_eq!(folder, Some(personal.id), "the query stays where it was");
    Ok(())
}

/// An edit holder's query move is refused wherever it goes: the shared
/// root, a sibling folder of the same space, and a folder of another space.
/// The row does not change in any of the three (`QS-3` review of #382).
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_holders_query_move_is_refused_to_the_root_a_sibling_and_another_space(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let weekly = create_folder(&pool, tenant_id, Some(person(RINA)), "Weekly", None)
        .await
        .unwrap();
    let sibling = create_folder(&pool, tenant_id, Some(person(RINA)), "Sibling", None)
        .await
        .unwrap();
    let saris_space = ensure_personal_space(&pool, tenant_id, person(SARI), "Sari")
        .await
        .unwrap();
    let saved = create_saved_query(
        &pool,
        &rina,
        "Rina's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(weekly.id),
    )
    .await
    .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();

    for (place, target) in [
        ("the shared root", FolderMove::Root),
        (
            "a sibling folder of the same space",
            FolderMove::Into(sibling.id),
        ),
        (
            "a folder of another space",
            FolderMove::Into(saris_space.id),
        ),
    ] {
        let refused = update_saved_query(
            &pool,
            &sari,
            &saved.id,
            &SavedQueryPatch {
                folder: Some(target),
                ..Default::default()
            },
        )
        .await;
        assert!(
            matches!(refused, Err(lakehouse_store::StoreError::NotAllowed)),
            "an edit holder cannot move the query to {place}: {refused:?}"
        );
        let folder: Option<Uuid> =
            sqlx::query_scalar("SELECT folder_id FROM saved_query WHERE id = $1")
                .bind(query_id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            folder,
            Some(weekly.id),
            "the row stays after a move to {place}"
        );
    }
    Ok(())
}

/// A manager's move inside one space succeeds, to a folder and to the
/// root.
#[sqlx::test(migrations = "../../migrations")]
async fn a_managers_move_inside_one_space_succeeds(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let weekly = create_folder(&pool, tenant_id, Some(person(RINA)), "Weekly", None)
        .await
        .unwrap();
    let saved = create_saved_query(
        &pool,
        &rina,
        "Rina's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(weekly.id),
    )
    .await
    .unwrap();
    replace_grants(
        &pool,
        Some(Uuid::parse_str(&saved.id).expect("the saved query id")),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Manage,
        }],
        None,
    )
    .await
    .unwrap();

    let to_root = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            folder: Some(FolderMove::Root),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(to_root.folder_id, None, "the manager moves it to the root");
    let back = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            folder: Some(FolderMove::Into(weekly.id)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        back.folder_id,
        Some(weekly.id.to_string()),
        "the manager moves it into a folder"
    );
    Ok(())
}

/// A PATCH that names the folder the query is already in is not a move: an
/// edit holder who renames with the folder re-sent still succeeds, for a
/// query in a folder and for a query at the root.
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_holders_patch_that_names_the_current_folder_is_not_a_move(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let weekly = create_folder(&pool, tenant_id, Some(person(RINA)), "Weekly", None)
        .await
        .unwrap();
    let in_folder = create_saved_query(
        &pool,
        &rina,
        "In a folder",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(weekly.id),
    )
    .await
    .unwrap();
    let at_root = create_saved_query_at(
        &pool,
        &rina,
        "At the root",
        "SELECT 2",
        "Rina Wijaya",
        &[],
        SavedQueryPlacement::SharedSpace,
    )
    .await
    .unwrap();
    for saved in [&in_folder, &at_root] {
        replace_grants(
            &pool,
            Some(Uuid::parse_str(&saved.id).expect("the saved query id")),
            None,
            &[NewGrant {
                user_id: Some(person(SARI)),
                role_id: None,
                level: Level::Edit,
            }],
            None,
        )
        .await
        .unwrap();
    }

    let renamed = update_saved_query(
        &pool,
        &sari,
        &in_folder.id,
        &SavedQueryPatch {
            title: Some("Renamed".to_owned()),
            folder: Some(FolderMove::Into(weekly.id)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        renamed.is_ok(),
        "the folder the query is in, sent again, is no move: {renamed:?}"
    );
    assert_eq!(renamed.unwrap().title, "Renamed");

    let renamed = update_saved_query(
        &pool,
        &sari,
        &at_root.id,
        &SavedQueryPatch {
            title: Some("Renamed too".to_owned()),
            folder: Some(FolderMove::Root),
            ..Default::default()
        },
    )
    .await;
    assert!(
        renamed.is_ok(),
        "the root, sent again for a query at the root, is no move: {renamed:?}"
    );
    Ok(())
}

/// A move into a folder of another tenant than the query's is refused as
/// an unseen folder — even for a caller that sees every tenant's rows,
/// and even for a caller of the tenant the query belongs to.
#[sqlx::test(migrations = "../../migrations")]
async fn a_move_into_another_tenants_folder_is_refused_as_an_unseen_folder(
    pool: PgPool,
) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let retail = tenant(RETAIL_TENANT);
    let rina = viewer(Some(person(RINA)), Some(group));
    // The query sits in the shared space, so only the tenant check can
    // hide the folder from the move.
    let saved = create_saved_query_at(
        &pool,
        &rina,
        "The group's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        SavedQueryPlacement::SharedSpace,
    )
    .await
    .unwrap();
    let theirs = create_folder(&pool, retail, None, "Theirs", None)
        .await
        .unwrap();
    let query_id = Uuid::parse_str(&saved.id).expect("the saved query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();

    // A caller that acts as no tenant in particular reaches every row,
    // and the folder of another tenant is still no place to move into.
    let wildcard = viewer(Some(person(SARI)), None);
    let refused = update_saved_query(
        &pool,
        &wildcard,
        &saved.id,
        &SavedQueryPatch {
            folder: Some(FolderMove::Into(theirs.id)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(refused, Err(lakehouse_store::StoreError::NotFound)),
        "another tenant's folder is not found at all: {refused:?}"
    );

    // A caller of the tenant the query belongs to is refused the same
    // way.
    let sari = viewer(Some(person(SARI)), Some(group));
    let refused = update_saved_query(
        &pool,
        &sari,
        &saved.id,
        &SavedQueryPatch {
            folder: Some(FolderMove::Into(theirs.id)),
            ..Default::default()
        },
    )
    .await;
    assert!(
        matches!(refused, Err(lakehouse_store::StoreError::NotFound)),
        "another tenant's folder is not found for the tenant either: {refused:?}"
    );
    Ok(())
}

/// A rule names a person or a role, and one item: never two of either.
#[sqlx::test(migrations = "../../migrations")]
async fn a_rule_names_a_person_or_a_role_and_never_both(pool: PgPool) -> sqlx::Result<()> {
    let query_id = Uuid::parse_str(SEEDED_QUERY).expect("a seeded query id");
    let folder_id = Uuid::new_v4();

    let both_people = replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: Some(person(RINA)),
            role_id: Some(Uuid::parse_str(ANALYST_ROLE).expect("a seeded role id")),
            level: Level::Edit,
        }],
        None,
    )
    .await;
    assert!(
        matches!(both_people, Err(lakehouse_store::StoreError::Validation(_))),
        "a rule names a person or a role, never both"
    );

    let neither = replace_grants(
        &pool,
        Some(query_id),
        None,
        &[NewGrant {
            user_id: None,
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await;
    assert!(
        matches!(neither, Err(lakehouse_store::StoreError::Validation(_))),
        "a rule names somebody"
    );

    let both_items = replace_grants(
        &pool,
        Some(query_id),
        Some(folder_id),
        &[NewGrant {
            user_id: Some(person(RINA)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await;
    assert!(
        matches!(both_items, Err(lakehouse_store::StoreError::Validation(_))),
        "a rule is on one item, never two"
    );
    Ok(())
}

/// The rules that name the viewer are the ones that count: a person's own
/// rule, and the rules of the roles they hold.
#[sqlx::test(migrations = "../../migrations")]
async fn the_rules_of_a_person_and_of_a_role_are_read_for_the_viewer(
    pool: PgPool,
) -> sqlx::Result<()> {
    let query_id = Uuid::parse_str(SEEDED_QUERY).expect("a seeded query id");
    replace_grants(
        &pool,
        Some(query_id),
        None,
        &[
            NewGrant {
                user_id: Some(person(RINA)),
                role_id: None,
                level: Level::Edit,
            },
            NewGrant {
                user_id: None,
                role_id: Some(Uuid::parse_str(ANALYST_ROLE).expect("a seeded role id")),
                level: Level::Manage,
            },
        ],
        None,
    )
    .await
    .unwrap();

    let own = grants_for_viewer(&pool, &viewer(Some(person(RINA)), None))
        .await
        .unwrap();
    assert_eq!(own.len(), 1, "the person's own rule is read");
    assert_eq!(own[0].level, Level::Edit);

    let by_role = SavedQueryViewer {
        id: Some(person(SARI)),
        role_names: vec!["Analyst".to_owned()],
        tenant_id: None,
        all: false,
    };
    let through_role = grants_for_viewer(&pool, &by_role).await.unwrap();
    assert_eq!(
        through_role.len(),
        1,
        "the role's rule is read for its holders"
    );
    assert_eq!(through_role[0].level, Level::Manage);

    let on_item = rules_for_item(&pool, Some(query_id), None).await.unwrap();
    assert_eq!(on_item.len(), 2, "the item carries both of its rules");
    Ok(())
}

// ── The folder writes and the grantee search ─────────────────────────────

/// A folder is made at the root and under a parent, and comes back with
/// the name, the parent and the owner it was given.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_is_created_at_the_root_and_under_a_parent(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);

    let root = create_folder(&pool, tenant_id, Some(person(RINA)), "Reports", None)
        .await
        .unwrap();
    assert_eq!(root.name, "Reports");
    assert_eq!(root.parent_id, None, "it sits at the root");
    assert_eq!(root.tenant_id, tenant_id);
    assert_eq!(root.owner_id, Some(person(RINA)));
    assert!(
        !root.is_personal,
        "an ordinary folder is not a personal space"
    );

    let child = create_folder(
        &pool,
        tenant_id,
        Some(person(RINA)),
        "Weekly",
        Some(root.id),
    )
    .await
    .unwrap();
    assert_eq!(child.name, "Weekly");
    assert_eq!(child.parent_id, Some(root.id), "it sits under its parent");
    Ok(())
}

/// A name is 1 to 100 characters: a blank one and one that is too long
/// are refused, and exactly a hundred is accepted.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_name_must_have_between_one_and_a_hundred_characters(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);

    let blank = create_folder(&pool, tenant_id, None, "   ", None).await;
    assert!(
        matches!(blank, Err(FolderError::InvalidName)),
        "a name of only whitespace is refused"
    );

    let too_long = create_folder(&pool, tenant_id, None, &"x".repeat(101), None).await;
    assert!(
        matches!(too_long, Err(FolderError::InvalidName)),
        "a name of 101 characters is refused"
    );

    let exact = create_folder(&pool, tenant_id, None, &"x".repeat(100), None)
        .await
        .unwrap();
    assert_eq!(exact.name.chars().count(), 100);
    Ok(())
}

/// A sibling's name is unique, compared case-insensitively: "Reports" and
/// "reports" are the same name.
#[sqlx::test(migrations = "../../migrations")]
async fn a_sibling_name_is_refused_case_insensitively(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    create_folder(&pool, tenant_id, None, "Reports", None)
        .await
        .unwrap();

    let shouted = create_folder(&pool, tenant_id, None, "reports", None).await;
    assert!(
        matches!(shouted, Err(FolderError::DuplicateName)),
        "the same name in another case is the same name"
    );
    Ok(())
}

/// A name that is free among the siblings is accepted, even when a
/// folder with that name sits somewhere else in the tree.
#[sqlx::test(migrations = "../../migrations")]
async fn a_name_free_among_the_siblings_is_accepted_even_when_it_exists_elsewhere(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    create_folder(&pool, tenant_id, None, "Reports", None)
        .await
        .unwrap();
    let other = create_folder(&pool, tenant_id, None, "Other", None)
        .await
        .unwrap();

    let elsewhere = create_folder(&pool, tenant_id, None, "Reports", Some(other.id))
        .await
        .unwrap();
    assert_eq!(
        elsewhere.parent_id,
        Some(other.id),
        "the same name under another parent is another name"
    );
    Ok(())
}

/// A personal space carries a name the system wrote, and it blocks
/// nobody: an ordinary folder may carry the same name.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_space_does_not_block_the_name_of_an_ordinary_folder(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();

    let ordinary = create_folder(
        &pool,
        tenant_id,
        None,
        "Personal space of Rina Wijaya",
        None,
    )
    .await
    .unwrap();
    assert_eq!(ordinary.name, "Personal space of Rina Wijaya");
    Ok(())
}

/// A personal space is one person's own space: it is not renamed, not
/// moved and not deleted.
#[sqlx::test(migrations = "../../migrations")]
async fn a_personal_space_is_not_renamed_not_moved_and_not_deleted(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let space = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    assert!(space.is_personal);

    let renamed = rename_folder(&pool, space.id, "Mine").await;
    assert!(
        matches!(renamed, Err(FolderError::PersonalSpace)),
        "a personal space is not renamed"
    );

    let moved =
        move_folder_for_viewer(&pool, &manager_of_everything(tenant_id), space.id, None).await;
    assert!(
        matches!(moved, Err(FolderError::PersonalSpace)),
        "a personal space is not moved"
    );

    let deleted = delete_folder(&pool, space.id).await;
    assert!(
        matches!(deleted, Err(FolderError::PersonalSpace)),
        "a personal space is not deleted"
    );

    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM saved_query_folder WHERE id = $1")
        .bind(space.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 1, "the personal space is still there");
    Ok(())
}

/// A folder may not move into itself or into one of its own descendants.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_may_not_move_into_its_own_descendant(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let parent = create_folder(&pool, tenant_id, None, "Parent", None)
        .await
        .unwrap();
    let child = create_folder(&pool, tenant_id, None, "Child", Some(parent.id))
        .await
        .unwrap();
    let grandchild = create_folder(&pool, tenant_id, None, "Grandchild", Some(child.id))
        .await
        .unwrap();

    let everything = manager_of_everything(tenant_id);
    let into_itself = move_folder_for_viewer(&pool, &everything, parent.id, Some(parent.id)).await;
    assert!(
        matches!(into_itself, Err(FolderError::Cycle)),
        "a folder is not its own parent"
    );

    let into_a_descendant =
        move_folder_for_viewer(&pool, &everything, parent.id, Some(grandchild.id)).await;
    assert!(
        matches!(into_a_descendant, Err(FolderError::Cycle)),
        "a folder is not moved under its own descendant"
    );
    Ok(())
}

/// The tree is at most four levels deep: a folder that would make it five
/// is refused, and a move that keeps it at four is allowed.
#[sqlx::test(migrations = "../../migrations")]
async fn a_move_that_would_make_the_tree_five_deep_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let one = create_folder(&pool, tenant_id, None, "One", None)
        .await
        .unwrap();
    let two = create_folder(&pool, tenant_id, None, "Two", Some(one.id))
        .await
        .unwrap();
    let three = create_folder(&pool, tenant_id, None, "Three", Some(two.id))
        .await
        .unwrap();
    let four = create_folder(&pool, tenant_id, None, "Four", Some(three.id))
        .await
        .unwrap();

    let five = create_folder(&pool, tenant_id, None, "Five", Some(four.id)).await;
    assert!(
        matches!(five, Err(FolderError::TooDeep)),
        "the tree is at most four levels deep"
    );

    // A subtree of two levels: it may not land under the fourth level
    // (it would reach the sixth), and it may land under the second.
    let branch = create_folder(&pool, tenant_id, None, "Branch", None)
        .await
        .unwrap();
    create_folder(&pool, tenant_id, None, "Leaf", Some(branch.id))
        .await
        .unwrap();

    let everything = manager_of_everything(tenant_id);
    let too_deep = move_folder_for_viewer(&pool, &everything, branch.id, Some(four.id)).await;
    assert!(
        matches!(too_deep, Err(FolderError::TooDeep)),
        "the subtree would reach past the limit"
    );

    let allowed = move_folder_for_viewer(&pool, &everything, branch.id, Some(two.id))
        .await
        .unwrap();
    assert_eq!(allowed.parent_id, Some(two.id));
    Ok(())
}

/// A folder is deleted only when it holds nothing: not a live query, not
/// a trashed one and not a sub-folder.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_is_deleted_only_when_it_holds_nothing(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let caller = viewer(Some(person(RINA)), Some(tenant_id));

    let with_live = create_folder(&pool, tenant_id, None, "Live", None)
        .await
        .unwrap();
    let live = create_saved_query(
        &pool,
        &caller,
        "Live one",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(with_live.id),
    )
    .await
    .unwrap();
    assert_eq!(
        live.folder_id,
        Some(with_live.id.to_string()),
        "the query sits in the folder that holds it"
    );
    let refused = delete_folder(&pool, with_live.id).await;
    assert!(
        matches!(refused, Err(FolderError::HoldsQueries)),
        "a live query holds the folder"
    );

    let with_trash = create_folder(&pool, tenant_id, None, "Trash", None)
        .await
        .unwrap();
    let trashed = create_saved_query(
        &pool,
        &caller,
        "Trashed one",
        "SELECT 2",
        "Rina Wijaya",
        &[],
        Some(with_trash.id),
    )
    .await
    .unwrap();
    trash_saved_query(&pool, &caller, &trashed.id)
        .await
        .unwrap();
    let refused = delete_folder(&pool, with_trash.id).await;
    assert!(
        matches!(refused, Err(FolderError::HoldsTrashedQueries)),
        "a query in the trash holds the folder too"
    );

    let with_child = create_folder(&pool, tenant_id, None, "Parent", None)
        .await
        .unwrap();
    create_folder(&pool, tenant_id, None, "Child", Some(with_child.id))
        .await
        .unwrap();
    let refused = delete_folder(&pool, with_child.id).await;
    assert!(
        matches!(refused, Err(FolderError::HoldsFolders)),
        "a sub-folder holds the folder"
    );

    let empty = create_folder(&pool, tenant_id, None, "Empty", None)
        .await
        .unwrap();
    delete_folder(&pool, empty.id).await.unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM saved_query_folder WHERE id = $1")
        .bind(empty.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(left, 0, "the empty folder is gone");
    Ok(())
}

/// Another tenant's folder is not reachable: it cannot be the parent of
/// a folder and it cannot be the place a folder moves into.
#[sqlx::test(migrations = "../../migrations")]
async fn another_tenants_folder_is_not_reachable(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let retail = tenant(RETAIL_TENANT);
    let theirs = create_folder(&pool, retail, None, "Theirs", None)
        .await
        .unwrap();

    let under_theirs = create_folder(&pool, group, None, "Mine", Some(theirs.id)).await;
    assert!(
        matches!(under_theirs, Err(FolderError::NotFound)),
        "the parent of another tenant is not found"
    );

    let mine = create_folder(&pool, group, None, "Mine", None)
        .await
        .unwrap();
    let into_theirs = move_folder_for_viewer(
        &pool,
        &manager_of_everything(group),
        mine.id,
        Some(theirs.id),
    )
    .await;
    assert!(
        matches!(into_theirs, Err(FolderError::NotFound)),
        "another tenant's folder is not a place to move into"
    );
    Ok(())
}

/// An edit grantee on a folder cannot move it across a space: a folder of
/// Rina's personal space does not move to the shared root on an edit rule
/// alone, and the folder stays where it was.
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_grantees_folder_move_across_a_space_is_refused(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let personal = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    let week = create_folder(&pool, tenant_id, None, "Week", Some(personal.id))
        .await
        .unwrap();
    replace_grants(
        &pool,
        None,
        Some(week.id),
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let refused = move_folder_for_viewer(&pool, &sari, week.id, None).await;
    assert!(
        matches!(refused, Err(FolderError::MoveNeedsManage)),
        "an edit grantee cannot move a folder across a space: {refused:?}"
    );
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM saved_query_folder WHERE id = $1")
            .bind(week.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        parent,
        Some(personal.id),
        "the folder stays in the personal space"
    );
    Ok(())
}

/// The owner of a personal space manages what is inside it: Rina moves a
/// folder of her space to the shared root.
#[sqlx::test(migrations = "../../migrations")]
async fn a_managers_folder_move_across_a_space_succeeds(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let personal = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    let week = create_folder(&pool, tenant_id, None, "Week", Some(personal.id))
        .await
        .unwrap();
    let rina = viewer(Some(person(RINA)), Some(tenant_id));

    let moved = move_folder_for_viewer(&pool, &rina, week.id, None).await;
    assert!(
        moved.is_ok(),
        "the space's owner moves the folder across: {moved:?}"
    );
    assert_eq!(
        moved.unwrap().parent_id,
        None,
        "the folder sits at the root"
    );
    Ok(())
}

/// A folder move inside one space needs manage too: a folder of Rina's
/// personal space does not move under another folder of that space on an
/// edit rule. This test used to expect success (`QS-3` review of #382, BLOCKER 1).
#[sqlx::test(migrations = "../../migrations")]
async fn an_edit_grantees_folder_move_inside_one_space_is_refused(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let personal = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    let week = create_folder(&pool, tenant_id, None, "Week", Some(personal.id))
        .await
        .unwrap();
    let notes = create_folder(&pool, tenant_id, None, "Notes", Some(personal.id))
        .await
        .unwrap();
    replace_grants(
        &pool,
        None,
        Some(week.id),
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Edit,
        }],
        None,
    )
    .await
    .unwrap();
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let refused = move_folder_for_viewer(&pool, &sari, week.id, Some(notes.id)).await;
    assert!(
        matches!(refused, Err(FolderError::MoveNeedsManage)),
        "a folder move inside one space needs manage too: {refused:?}"
    );
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM saved_query_folder WHERE id = $1")
            .bind(week.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(parent, Some(personal.id), "the folder stays where it was");
    Ok(())
}

/// A folder whose parent does not change is not moved: an edit holder who
/// sends the parent again is not refused.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_move_to_the_parent_it_already_has_is_no_move(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let personal = ensure_personal_space(&pool, tenant_id, person(RINA), "Rina Wijaya")
        .await
        .unwrap();
    let week = create_folder(&pool, tenant_id, None, "Week", Some(personal.id))
        .await
        .unwrap();
    let at_root = create_folder(&pool, tenant_id, None, "At the root", None)
        .await
        .unwrap();
    for folder in [week.id, at_root.id] {
        replace_grants(
            &pool,
            None,
            Some(folder),
            &[NewGrant {
                user_id: Some(person(SARI)),
                role_id: None,
                level: Level::Edit,
            }],
            None,
        )
        .await
        .unwrap();
    }
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    let same = move_folder_for_viewer(&pool, &sari, week.id, Some(personal.id)).await;
    assert!(same.is_ok(), "the same parent is no move: {same:?}");
    let same = move_folder_for_viewer(&pool, &sari, at_root.id, None).await;
    assert!(same.is_ok(), "the root, again, is no move: {same:?}");
    Ok(())
}

/// A folder tree of the `QS-3` review of #382: Sari wrote `X` at the root, so she
/// manages it, and `X` holds what Rina wrote.
struct CarriedTree {
    x: Uuid,
    /// A root folder of Sari's the move goes under.
    target: Uuid,
}

async fn carried_tree(pool: &PgPool, tenant_id: Uuid, suffix: &str) -> CarriedTree {
    let x = create_folder(
        pool,
        tenant_id,
        Some(person(SARI)),
        &format!("X {suffix}"),
        None,
    )
    .await
    .unwrap();
    let target = create_folder(
        pool,
        tenant_id,
        Some(person(SARI)),
        &format!("Target {suffix}"),
        None,
    )
    .await
    .unwrap();
    CarriedTree {
        x: x.id,
        target: target.id,
    }
}

/// The author of a folder does not manage what others put inside it: the
/// move is refused when the folder holds a query the mover does not
/// manage, live or in the trash, and the folder stays.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_move_is_refused_when_it_holds_a_query_the_mover_does_not_manage(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    for trashed in [false, true] {
        let tree = carried_tree(&pool, tenant_id, &format!("trashed {trashed}")).await;
        let saved = create_saved_query(
            &pool,
            &rina,
            "Rina's",
            "SELECT 1",
            "Rina Wijaya",
            &[],
            Some(tree.x),
        )
        .await
        .unwrap();
        if trashed {
            trash_saved_query(&pool, &rina, &saved.id).await.unwrap();
        }

        let refused = move_folder_for_viewer(&pool, &sari, tree.x, Some(tree.target)).await;
        assert!(
            matches!(refused, Err(FolderError::MoveNeedsManage)),
            "a query of somebody else (in the trash: {trashed}) stops the move: {refused:?}"
        );
        let parent: Option<Uuid> =
            sqlx::query_scalar("SELECT parent_id FROM saved_query_folder WHERE id = $1")
                .bind(tree.x)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            parent, None,
            "the folder stays at the root (trash: {trashed})"
        );
    }
    Ok(())
}

/// The rule reaches every depth: a sub-folder of somebody else's, and a
/// query of somebody else's three levels down, each stop the move even
/// when the folders in between are the mover's own.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_move_is_refused_for_what_is_deep_inside_it(pool: PgPool) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let tree = carried_tree(&pool, tenant_id, "case").await;
    // `X` and `Y` are Sari's: a folder of Rina's sits one level lower.
    let y = create_folder(&pool, tenant_id, Some(person(SARI)), "Y", Some(tree.x))
        .await
        .unwrap();
    let z = create_folder(&pool, tenant_id, Some(person(RINA)), "Z", Some(y.id))
        .await
        .unwrap();

    let refused = move_folder_for_viewer(&pool, &sari, tree.x, Some(tree.target)).await;
    assert!(
        matches!(refused, Err(FolderError::MoveNeedsManage)),
        "a sub-folder of somebody else's, two levels down, stops the move: {refused:?}"
    );

    // Hand `Z` to Sari: what stops the move now is a query of Rina's in it.
    sqlx::query("UPDATE saved_query_folder SET owner_id = $2 WHERE id = $1")
        .bind(z.id)
        .bind(person(SARI))
        .execute(&pool)
        .await?;
    let deep = create_saved_query(
        &pool,
        &rina,
        "Rina's, deep",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(z.id),
    )
    .await
    .unwrap();
    let refused = move_folder_for_viewer(&pool, &sari, tree.x, Some(tree.target)).await;
    assert!(
        matches!(refused, Err(FolderError::MoveNeedsManage)),
        "a query of somebody else's, three levels down, stops the move: {refused:?}"
    );

    // Rina trashes it: it is still carried.
    trash_saved_query(&pool, &rina, &deep.id).await.unwrap();
    let refused = move_folder_for_viewer(&pool, &sari, tree.x, Some(tree.target)).await;
    assert!(
        matches!(refused, Err(FolderError::MoveNeedsManage)),
        "a trashed query of somebody else's still stops the move: {refused:?}"
    );
    let parent: Option<Uuid> =
        sqlx::query_scalar("SELECT parent_id FROM saved_query_folder WHERE id = $1")
            .bind(tree.x)
            .fetch_one(&pool)
            .await?;
    assert_eq!(parent, None, "nothing moved");
    Ok(())
}

/// A folder move succeeds when the mover manages the folder, every
/// sub-folder and every query under it, the ones in the trash included.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_move_succeeds_when_the_mover_manages_everything_in_it(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));
    let tree = carried_tree(&pool, tenant_id, "case").await;
    let y = create_folder(&pool, tenant_id, Some(person(SARI)), "Y", Some(tree.x))
        .await
        .unwrap();
    create_saved_query(&pool, &sari, "Sari's", "SELECT 1", "Sari", &[], Some(y.id))
        .await
        .unwrap();
    let trashed = create_saved_query(
        &pool,
        &sari,
        "Sari's, trashed",
        "SELECT 2",
        "Sari",
        &[],
        Some(y.id),
    )
    .await
    .unwrap();
    trash_saved_query(&pool, &sari, &trashed.id).await.unwrap();
    // A query of Rina's inside, with a rule that gives Sari manage on it:
    // a rule is as good as authorship.
    let shared = create_saved_query(
        &pool,
        &rina,
        "Rina's, shared",
        "SELECT 3",
        "Rina Wijaya",
        &[],
        Some(tree.x),
    )
    .await
    .unwrap();
    replace_grants(
        &pool,
        Some(Uuid::parse_str(&shared.id).expect("the saved query id")),
        None,
        &[NewGrant {
            user_id: Some(person(SARI)),
            role_id: None,
            level: Level::Manage,
        }],
        None,
    )
    .await
    .unwrap();

    let moved = move_folder_for_viewer(&pool, &sari, tree.x, Some(tree.target)).await;
    assert!(moved.is_ok(), "the mover manages all of it: {moved:?}");
    assert_eq!(moved.unwrap().parent_id, Some(tree.target));
    Ok(())
}

/// Sharing a folder is held to the bar of a move (`QS-3` review of #393):
/// the viewer manages the folder, every sub-folder and every query under
/// it, the ones in the trash included. An own tree answers `true`; a live
/// query, a trashed query and an empty sub-folder of somebody else's each
/// answer `false`, and a folder that is not there answers `false`.
#[sqlx::test(migrations = "../../migrations")]
async fn a_folder_and_its_contents_are_managed_only_when_every_part_is(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_id = tenant(GROUP_TENANT);
    let rina = viewer(Some(person(RINA)), Some(tenant_id));
    let sari = viewer(Some(person(SARI)), Some(tenant_id));

    // Own tree: a sub-folder, a live query and a trashed query, all Sari's.
    let own = carried_tree(&pool, tenant_id, "own").await;
    let inner = create_folder(&pool, tenant_id, Some(person(SARI)), "Inner", Some(own.x))
        .await
        .unwrap();
    create_saved_query(
        &pool,
        &sari,
        "Live",
        "SELECT 1",
        "Sari",
        &[],
        Some(inner.id),
    )
    .await
    .unwrap();
    let trashed = create_saved_query(&pool, &sari, "Gone", "SELECT 2", "Sari", &[], Some(own.x))
        .await
        .unwrap();
    trash_saved_query(&pool, &sari, &trashed.id).await.unwrap();
    assert!(
        manages_folder_and_contents(&pool, &sari, own.x)
            .await
            .unwrap(),
        "Sari manages every part of her own tree"
    );

    // A live query of Rina's.
    let live = carried_tree(&pool, tenant_id, "live").await;
    create_saved_query(
        &pool,
        &rina,
        "Rina's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(live.x),
    )
    .await
    .unwrap();
    assert!(
        !manages_folder_and_contents(&pool, &sari, live.x)
            .await
            .unwrap(),
        "a live query of somebody else's is not managed"
    );

    // A trashed query of Rina's.
    let gone = carried_tree(&pool, tenant_id, "gone").await;
    let rinas = create_saved_query(
        &pool,
        &rina,
        "Rina's, trashed",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        Some(gone.x),
    )
    .await
    .unwrap();
    trash_saved_query(&pool, &rina, &rinas.id).await.unwrap();
    assert!(
        !manages_folder_and_contents(&pool, &sari, gone.x)
            .await
            .unwrap(),
        "a trashed query of somebody else's is not managed"
    );

    // An empty sub-folder of Rina's.
    let sub = carried_tree(&pool, tenant_id, "sub").await;
    create_folder(&pool, tenant_id, Some(person(RINA)), "Hers", Some(sub.x))
        .await
        .unwrap();
    assert!(
        !manages_folder_and_contents(&pool, &sari, sub.x)
            .await
            .unwrap(),
        "an empty sub-folder of somebody else's is not managed"
    );

    assert!(
        !manages_folder_and_contents(&pool, &sari, Uuid::new_v4())
            .await
            .unwrap(),
        "a folder that is not there is not managed"
    );
    Ok(())
}

/// The grantee search finds a person by their name and by their email.
#[sqlx::test(migrations = "../../migrations")]
async fn the_grantee_search_finds_a_person_by_name_and_by_email(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);

    let by_name = search_grantees(&pool, Some(group), "Rina", 20)
        .await
        .unwrap();
    assert!(
        by_name.people.iter().any(|grantee| {
            grantee.name == "Rina Wijaya"
                && grantee.email.as_deref() == Some("rina@meridian.example")
        }),
        "the person is found by their name"
    );

    let by_email = search_grantees(&pool, Some(group), "rina@meridian.example", 20)
        .await
        .unwrap();
    assert!(
        by_email
            .people
            .iter()
            .any(|grantee| grantee.id == person(RINA)),
        "the person is found by their email"
    );
    Ok(())
}

/// The grantee search finds a role by its name: a role belongs to the
/// deployment, so every role is a possible grantee.
#[sqlx::test(migrations = "../../migrations")]
async fn the_grantee_search_finds_a_role_by_name(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);

    let matches = search_grantees(&pool, Some(group), "Analyst", 20)
        .await
        .unwrap();
    assert!(
        matches.roles.iter().any(|role| role.name == "Analyst"),
        "the role is found"
    );
    assert!(
        matches.roles.iter().all(|role| !role.name.is_empty()),
        "a role carries its name"
    );
    Ok(())
}

/// The wildcards of the match are literals: a `%` is a percent sign and
/// an `_` is an underscore, never "anything".
#[sqlx::test(migrations = "../../migrations")]
async fn the_grantee_search_treats_the_wildcards_literally(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let person_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email) VALUES ($1, 'Fifty % Off', $2, $3)",
    )
    .bind(person_id)
    .bind("fifty")
    .bind("fifty@meridian.example")
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(person_id)
        .bind(group)
        .execute(&pool)
        .await?;

    let percent = search_grantees(&pool, Some(group), "%", 20).await.unwrap();
    assert!(
        percent
            .people
            .iter()
            .all(|grantee| grantee.name.contains('%')),
        "a percent matches a percent and nothing else"
    );
    assert!(
        percent.people.iter().any(|grantee| grantee.id == person_id),
        "the person whose name holds the percent is found"
    );

    let underscore = search_grantees(&pool, Some(group), "_", 20).await.unwrap();
    assert!(
        underscore.people.is_empty(),
        "an underscore is a literal underscore"
    );
    Ok(())
}

/// The grantee search answers with the people of the tenant and nobody
/// else: another tenant's people are not grantees, and an inactive
/// person is nobody.
#[sqlx::test(migrations = "../../migrations")]
async fn the_grantee_search_answers_with_the_tenants_people_only(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let inactive = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) \
         VALUES ($1, 'Sleepy Person', $2, $3, 'inactive')",
    )
    .bind(inactive)
    .bind(format!("sleepy-{inactive}"))
    .bind("sleepy@meridian.example")
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(inactive)
        .bind(group)
        .execute(&pool)
        .await?;

    let matches = search_grantees(&pool, Some(group), "", 20).await.unwrap();
    assert!(
        matches
            .people
            .iter()
            .any(|grantee| grantee.id == person(RINA)),
        "a member of the tenant is there"
    );
    assert!(
        !matches
            .people
            .iter()
            .any(|grantee| grantee.id == person(SARI)),
        "a member of another tenant is not"
    );
    assert!(
        !matches.people.iter().any(|grantee| grantee.id == inactive),
        "an inactive person is nobody"
    );

    let every_person = search_grantees(&pool, None, "", 20).await.unwrap();
    assert!(
        every_person
            .people
            .iter()
            .any(|grantee| grantee.id == person(SARI)),
        "without a tenant every person of the deployment is there"
    );

    let none = search_grantees(&pool, Some(group), "", 0).await.unwrap();
    assert!(none.people.is_empty(), "a limit of zero asks for no rows");
    Ok(())
}

/// A rule's grantee is named the way the share panel shows it: a person of
/// the viewer's tenant by name and email, a role by name.
#[sqlx::test(migrations = "../../migrations")]
async fn grantee_labels_name_a_person_of_the_tenant_and_a_role(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);

    let labels = grantee_labels(&pool, Some(group), &[person(RINA)], &[person(ANALYST_ROLE)])
        .await
        .unwrap();

    assert_eq!(labels.people.len(), 1, "{labels:?}");
    assert_eq!(labels.people[0].id, person(RINA));
    assert_eq!(labels.people[0].name, "Rina Wijaya");
    assert_eq!(
        labels.people[0].email.as_deref(),
        Some("rina@meridian.example")
    );
    assert_eq!(labels.roles.len(), 1, "{labels:?}");
    assert_eq!(labels.roles[0].id, person(ANALYST_ROLE));
    assert_eq!(labels.roles[0].name, "Analyst");
    Ok(())
}

/// A person who is not a member of the viewer's tenant gets no label: the
/// name and email of another organisation's person never reach the list.
#[sqlx::test(migrations = "../../migrations")]
async fn grantee_labels_leave_out_a_person_of_another_tenant(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);

    let labels = grantee_labels(&pool, Some(group), &[person(RINA), person(SARI)], &[])
        .await
        .unwrap();

    assert!(
        labels.people.iter().any(|found| found.id == person(RINA)),
        "CONTROL: the member of the tenant is named"
    );
    assert!(
        !labels.people.iter().any(|found| found.id == person(SARI)),
        "Sari belongs to the Retail tenant only: {labels:?}"
    );
    Ok(())
}

/// A rule still applies to a person who has been deactivated, so the list
/// keeps naming them, unlike the grantee search.
#[sqlx::test(migrations = "../../migrations")]
async fn grantee_labels_keep_the_name_of_an_inactive_person(pool: PgPool) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);
    let inactive = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) \
         VALUES ($1, 'Sleepy Person', $2, $3, 'inactive')",
    )
    .bind(inactive)
    .bind(format!("sleepy-{inactive}"))
    .bind("sleepy@meridian.example")
    .execute(&pool)
    .await?;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(inactive)
        .bind(group)
        .execute(&pool)
        .await?;

    let labels = grantee_labels(&pool, Some(group), &[inactive], &[])
        .await
        .unwrap();

    assert_eq!(labels.people.len(), 1, "{labels:?}");
    assert_eq!(labels.people[0].name, "Sleepy Person");
    assert_eq!(
        labels.people[0].email.as_deref(),
        Some("sleepy@meridian.example")
    );
    Ok(())
}

/// An id the database does not hold is absent from the answer, and an
/// empty input is an empty answer.
#[sqlx::test(migrations = "../../migrations")]
async fn grantee_labels_leave_out_an_unknown_id_and_answer_nothing_for_no_ids(
    pool: PgPool,
) -> sqlx::Result<()> {
    let group = tenant(GROUP_TENANT);

    let unknown = grantee_labels(
        &pool,
        Some(group),
        &[Uuid::new_v4(), person(RINA)],
        &[Uuid::new_v4()],
    )
    .await
    .unwrap();
    assert_eq!(unknown.people.len(), 1, "{unknown:?}");
    assert_eq!(unknown.people[0].id, person(RINA));
    assert!(unknown.roles.is_empty(), "{unknown:?}");

    let none = grantee_labels(&pool, Some(group), &[], &[]).await.unwrap();
    assert!(none.people.is_empty() && none.roles.is_empty(), "{none:?}");
    Ok(())
}

/// With no tenant every person of the deployment is named, the same
/// reading of "no tenant" as the grantee search and the tenant check.
#[sqlx::test(migrations = "../../migrations")]
async fn grantee_labels_with_no_tenant_name_every_person(pool: PgPool) -> sqlx::Result<()> {
    let labels = grantee_labels(&pool, None, &[person(RINA), person(SARI)], &[])
        .await
        .unwrap();

    assert_eq!(labels.people.len(), 2, "{labels:?}");
    assert!(labels.people.iter().any(|found| found.id == person(SARI)));
    Ok(())
}
