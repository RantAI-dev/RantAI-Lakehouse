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
    HistoryFilter, RecordHistoryInput, SavedQueryScope, create_saved_query, get_history_item,
    history_mentioning, history_recent, list_history, list_saved, record_history,
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
    let saved = list_saved(&pool, SavedQueryScope::All).await.unwrap();
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

    let groups = create_saved_query(
        &pool,
        "Group query",
        "SELECT 1",
        "Rina",
        &[],
        None,
        Some(group),
    )
    .await
    .unwrap();
    let retails = create_saved_query(
        &pool,
        "Retail query",
        "SELECT 2",
        "Sari",
        &[],
        None,
        Some(retail),
    )
    .await
    .unwrap();

    // Every row: the two seeded ones (no tenant), and the two written
    // here.
    let all = list_saved(&pool, SavedQueryScope::All).await.unwrap();
    assert_eq!(all.len(), 4);

    // One tenant's rows, and only that tenant's.
    let own = list_saved(
        &pool,
        SavedQueryScope::Tenant {
            tenant_id: group,
            include_tenantless: false,
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
