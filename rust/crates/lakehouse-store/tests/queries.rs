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
    HistoryFilter, RecordHistoryInput, SavedQueryPatch, SavedQueryScope, SavedQueryViewer,
    VERSION_KEEP, create_saved_query, get_history_item, get_saved, history_mentioning,
    history_recent, list_history, list_saved, list_saved_matching, list_saved_trash, list_versions,
    purge_trashed_before, record_history, restore_saved_query, trash_saved_query,
    update_saved_query,
};
use lakehouse_store::saved_query_access::{
    Level, NewGrant, grants_for_item, grants_for_viewer, replace_grants,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

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

/// The cursor pages by the `at` *and* `id` of the last row seen: 55 rows
/// that share one `at` come back as 50 then 5, with nothing repeated and
/// nothing skipped — the millisecond tie is what the id half is for.
#[sqlx::test(migrations = "../../migrations")]
async fn the_cursor_pages_without_repeats_or_skips(pool: PgPool) -> sqlx::Result<()> {
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
    let last = page_one.last().unwrap();
    let cursor_at =
        OffsetDateTime::parse(&last.at, &time::format_description::well_known::Rfc3339).unwrap();

    let page_two = list_history(
        &pool,
        owner,
        &HistoryFilter {
            before_at: Some(cursor_at),
            before_id: Some(&last.id),
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
/// across the page boundary.
///
/// The value a row's `at` leaves the store with is cut to milliseconds, so
/// the cursor the console carries for the last row of a page is that
/// millisecond. The page after it must compare on the same value: a
/// comparison against the stored microsecond `at` skips every row inside
/// the last page's millisecond, and those rows are never seen again.
///
/// The two rows are placed so their ids disagree with their microseconds:
/// `q-a` is the newer of the two by a microsecond and the older by id. The
/// order therefore has to cut to the millisecond and fall back to the id,
/// the way the comparison does. An order on the raw microsecond puts `q-a`
/// at the end of the page and leaves `q-b` behind the cursor, so `q-b` is
/// never seen again.
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
    // next — the boundary the console's cursor is built from.
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
    // there and leaves `q-b` behind the cursor.
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
    let cursor_at =
        OffsetDateTime::parse(&last.at, &time::format_description::well_known::Rfc3339).unwrap();
    assert!(
        last.at.ends_with('Z'),
        "the cursor carries an ISO timestamp: {}",
        last.at
    );
    assert_eq!(
        cursor_at.millisecond(),
        0,
        "the cursor carries whole milliseconds: {}",
        last.at
    );

    let page_two = list_history(
        &pool,
        owner,
        &HistoryFilter {
            before_at: Some(cursor_at),
            before_id: Some(&last.id),
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
        list_saved_trash(&pool, &scope)
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
        !list_saved_trash(&pool, &scope)
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
    let saved = create_saved_query(
        &pool,
        &rina,
        "The group's",
        "SELECT 1",
        "Rina Wijaya",
        &[],
        None,
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

    let on_item = grants_for_item(&pool, Some(query_id), None).await.unwrap();
    assert_eq!(on_item.len(), 2, "the item carries both of its rules");
    Ok(())
}
