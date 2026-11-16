//! Integration tests for `lakehouse_store::usage` against a real Postgres
//! (`#[sqlx::test]`, see `tests/maintenance_policy.rs` for how the database is
//! provided): `OPS-8` part B, the sums over `query_history` and the rates.
//!
//! The seam is the store's two public entry points, [`usage::report`] and the
//! rate functions. A run is inserted with the `at` a test wants, because the
//! store reads days, not "now".

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use std::time::Instant;

use lakehouse_store::usage::{self, Group, NewUsageRate, TenantScope, UsageQuery, UsageRow};
use sqlx::PgPool;
use time::{Date, Month};
use uuid::Uuid;

const TENANT_A: &str = "11111111-1111-4111-8111-000000000001";
const TENANT_B: &str = "11111111-1111-4111-8111-000000000002";

fn day(month: u8, d: u8) -> Date {
    Date::from_calendar_date(2026, Month::try_from(month).unwrap(), d).unwrap()
}

fn tenant(id: &str) -> Uuid {
    id.parse().unwrap()
}

/// One run, as the columns a sum reads. `at` is a UTC timestamp literal.
struct Run<'a> {
    at: &'a str,
    status: &'a str,
    engine: &'a str,
    owner: Option<Uuid>,
    user: &'a str,
    tenant: Option<&'a str>,
    scanned_bytes: i64,
    duration_ms: i64,
}

impl<'a> Run<'a> {
    fn new(at: &'a str) -> Self {
        Self {
            at,
            status: "completed",
            engine: "clickhouse",
            owner: None,
            user: "Rina",
            tenant: Some(TENANT_A),
            scanned_bytes: 0,
            duration_ms: 0,
        }
    }
}

async fn insert(pool: &PgPool, run: &Run<'_>) {
    sqlx::query(
        "INSERT INTO query_history \
         (id, sql, user_name, at, status, duration_ms, scanned_bytes, cost_units, \
          workload_class, engine, owner_id, tenant_id) \
         VALUES ($1, 'SELECT 1', $2, $3::timestamptz, $4, $5, $6, 0, 'hot-analytics', $7, $8, $9)",
    )
    .bind(format!("q-{}", Uuid::new_v4()))
    .bind(run.user)
    .bind(run.at)
    .bind(run.status)
    .bind(run.duration_ms)
    .bind(run.scanned_bytes)
    .bind(run.engine)
    .bind(run.owner)
    .bind(run.tenant.map(tenant))
    .execute(pool)
    .await
    .expect("insert a query_history row");
}

fn query(from: Date, to: Date, group: Group, scope: TenantScope) -> UsageQuery {
    UsageQuery {
        from,
        to,
        group,
        scope,
    }
}

fn row<'a>(rows: &'a [UsageRow], key: &str) -> &'a UsageRow {
    rows.iter()
        .find(|r| r.key == key)
        .unwrap_or_else(|| panic!("no row for {key}: {rows:?}"))
}

async fn rate(pool: &PgPool, kind: &str, amount: &str, from: Date) {
    let added = usage::add_rate(
        pool,
        &NewUsageRate {
            kind,
            currency: "USD",
            amount,
            effective_from: from,
            created_by: "test",
        },
    )
    .await
    .unwrap();
    assert!(added.is_some(), "the rate must be accepted");
}

// ── the table's own checks (step 2) ──────────────────────────────────────

async fn raw_rate(
    pool: &PgPool,
    kind: &str,
    currency: &str,
    amount: &str,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO usage_rate (kind, currency, amount, effective_from, created_by) \
         VALUES ($1, $2, $3::numeric, DATE '2026-03-01', 'test')",
    )
    .bind(kind)
    .bind(currency)
    .bind(amount)
    .execute(pool)
    .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_database_refuses_a_negative_amount_a_bad_currency_and_an_unknown_kind(pool: PgPool) {
    raw_rate(&pool, "scan_tb", "USD", "5.00").await.unwrap();
    assert!(raw_rate(&pool, "scan_tb", "USD", "-0.01").await.is_err());
    assert!(raw_rate(&pool, "scan_tb", "USDX", "1").await.is_err());
    assert!(raw_rate(&pool, "scan_tb", "usd", "1").await.is_err());
    assert!(raw_rate(&pool, "scan_tb", "US", "1").await.is_err());
    assert!(raw_rate(&pool, "per_query", "USD", "1").await.is_err());
    assert_eq!(usage::list_rates(&pool).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rate_row_is_append_only(pool: PgPool) {
    raw_rate(&pool, "scan_tb", "USD", "5.00").await.unwrap();
    let update = sqlx::query("UPDATE usage_rate SET amount = 9")
        .execute(&pool)
        .await;
    assert!(update.is_err(), "an UPDATE of a rate must be refused");
    let delete = sqlx::query("DELETE FROM usage_rate").execute(&pool).await;
    assert!(delete.is_err(), "a DELETE of a rate must be refused");
    let truncate = sqlx::query("TRUNCATE usage_rate").execute(&pool).await;
    assert!(truncate.is_err(), "a TRUNCATE of the rates must be refused");
    assert_eq!(usage::list_rates(&pool).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_rate_in_another_currency_than_the_ones_stored_is_not_added(pool: PgPool) {
    rate(&pool, "scan_tb", "5", day(3, 1)).await;
    let refused = usage::add_rate(
        &pool,
        &NewUsageRate {
            kind: "engine_hour",
            currency: "IDR",
            amount: "1",
            effective_from: day(3, 1),
            created_by: "test",
        },
    )
    .await
    .unwrap();
    assert!(refused.is_none(), "mixed currencies must be refused");
    let rates = usage::list_rates(&pool).await.unwrap();
    assert_eq!(rates.len(), 1);
    assert_eq!(rates[0].currency, "USD");
    assert_eq!(rates[0].kind, "scan_tb");
}

// ── the sums (step 3) ────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn a_caller_of_one_tenant_sums_none_of_the_others_rows(pool: PgPool) {
    let mut a = Run::new("2026-03-02T10:00:00Z");
    a.scanned_bytes = 100;
    a.user = "Alice of A";
    insert(&pool, &a).await;
    let mut b = Run::new("2026-03-02T11:00:00Z");
    b.tenant = Some(TENANT_B);
    b.scanned_bytes = 7;
    b.user = "Bob of B";
    insert(&pool, &b).await;
    let mut none = Run::new("2026-03-02T12:00:00Z");
    none.tenant = None;
    none.scanned_bytes = 1000;
    none.user = "Nobody";
    insert(&pool, &none).await;

    for group in [Group::Day, Group::Person, Group::Engine, Group::Tenant] {
        let report = usage::report(
            &pool,
            &query(
                day(3, 1),
                day(3, 31),
                group,
                TenantScope::Tenant(tenant(TENANT_B)),
            ),
        )
        .await
        .unwrap();
        assert_eq!(report.totals.queries, 1, "{group:?}: only B's run");
        assert_eq!(report.totals.scanned_bytes, 7, "{group:?}");
        let seen = format!("{:?}", report.rows);
        assert!(!seen.contains("Alice of A"), "{group:?}: {seen}");
        assert!(!seen.contains("Nobody"), "{group:?}: {seen}");
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn everyone_sees_every_tenant_and_the_rows_with_no_tenant_labelled_so(pool: PgPool) {
    let mut a = Run::new("2026-03-02T10:00:00Z");
    a.scanned_bytes = 100;
    insert(&pool, &a).await;
    let mut b = Run::new("2026-03-02T11:00:00Z");
    b.tenant = Some(TENANT_B);
    b.scanned_bytes = 7;
    insert(&pool, &b).await;
    let mut none = Run::new("2026-03-02T12:00:00Z");
    none.tenant = None;
    none.scanned_bytes = 1000;
    insert(&pool, &none).await;

    let report = usage::report(
        &pool,
        &query(day(3, 1), day(3, 31), Group::Tenant, TenantScope::Everyone),
    )
    .await
    .unwrap();
    assert_eq!(report.totals.queries, 3);
    assert_eq!(report.totals.scanned_bytes, 1107);
    assert_eq!(report.rows.len(), 3);
    let unknown = row(&report.rows, "none");
    assert_eq!(unknown.label, "no tenant");
    assert_eq!(unknown.scanned_bytes, 1000);
    let seeded_a: String = sqlx::query_scalar("SELECT name FROM tenant WHERE id = $1")
        .bind(tenant(TENANT_A))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row(&report.rows, TENANT_A).label, seeded_a);
    assert_eq!(row(&report.rows, TENANT_A).scanned_bytes, 100);
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_four_counts_add_up_and_a_stopped_run_counts_as_cancelled(pool: PgPool) {
    for (status, n) in [
        ("completed", 3),
        ("failed", 2),
        ("blocked", 1),
        ("cancelled", 1),
        ("stopped", 2),
    ] {
        for i in 0..n {
            let at = format!("2026-03-02T10:0{i}:00Z");
            let mut r = Run::new(&at);
            r.status = status;
            insert(&pool, &r).await;
        }
    }
    let report = usage::report(
        &pool,
        &query(
            day(3, 2),
            day(3, 2),
            Group::Day,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    let t = &report.totals;
    assert_eq!(t.queries, 9);
    assert_eq!(t.completed, 3);
    assert_eq!(t.failed, 2);
    assert_eq!(t.blocked, 1);
    assert_eq!(t.cancelled, 3, "1 cancelled + 2 stopped");
    assert_eq!(t.completed + t.failed + t.blocked + t.cancelled, t.queries);
    assert_eq!(row(&report.rows, "2026-03-02").cancelled, 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn days_are_utc_days_and_the_last_day_is_included(pool: PgPool) {
    for at in [
        "2026-03-01T23:59:59Z", // the day before the window
        "2026-03-02T00:00:00Z", // first instant of the first day
        "2026-03-03T23:59:59Z", // last instant of the last day
        "2026-03-04T00:00:00Z", // the day after
    ] {
        insert(&pool, &Run::new(at)).await;
    }
    let report = usage::report(
        &pool,
        &query(
            day(3, 2),
            day(3, 3),
            Group::Day,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    assert_eq!(report.totals.queries, 2);
    let keys: Vec<&str> = report.rows.iter().map(|r| r.key.as_str()).collect();
    assert_eq!(keys, ["2026-03-02", "2026-03-03"]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_person_is_the_owner_and_an_ownerless_run_groups_under_its_user_name(pool: PgPool) {
    let owner = Uuid::new_v4();
    for (at, user) in [
        ("2026-03-02T10:00:00Z", "Rina (old name)"),
        ("2026-03-02T11:00:00Z", "Rina"),
    ] {
        let mut r = Run::new(at);
        r.owner = Some(owner);
        r.user = user;
        r.scanned_bytes = 5;
        insert(&pool, &r).await;
    }
    for _ in 0..2 {
        let mut r = Run::new("2026-03-02T12:00:00Z");
        r.owner = None;
        r.user = "legacy-user";
        insert(&pool, &r).await;
    }
    let report = usage::report(
        &pool,
        &query(
            day(3, 2),
            day(3, 2),
            Group::Person,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    assert_eq!(report.rows.len(), 2);
    let person = row(&report.rows, &owner.to_string());
    assert_eq!(person.queries, 2, "one person, two names over time");
    assert_eq!(person.label, "Rina", "the name on the newest run");
    let legacy = row(&report.rows, "legacy-user");
    assert_eq!(legacy.queries, 2);
    assert_eq!(legacy.label, "legacy-user");
}

#[sqlx::test(migrations = "../../migrations")]
async fn with_no_rate_the_estimate_is_null_and_a_rate_applies_from_its_effective_date(
    pool: PgPool,
) {
    // One run of 10^12 bytes on each of days 2 and 4.
    for at in ["2026-03-02T10:00:00Z", "2026-03-04T10:00:00Z"] {
        let mut r = Run::new(at);
        r.scanned_bytes = 1_000_000_000_000;
        insert(&pool, &r).await;
    }
    let scope = || TenantScope::Tenant(tenant(TENANT_A));

    let none = usage::report(&pool, &query(day(3, 1), day(3, 5), Group::Day, scope()))
        .await
        .unwrap();
    assert_eq!(none.totals.estimate, None);
    assert!(
        none.rows.iter().all(|r| r.estimate.is_none()),
        "with no rate configured, no row has an estimate"
    );

    rate(&pool, "scan_tb", "5.00", day(3, 1)).await;
    rate(&pool, "scan_tb", "10.00", day(3, 3)).await;
    let rated = usage::report(&pool, &query(day(3, 1), day(3, 5), Group::Day, scope()))
        .await
        .unwrap();
    assert_eq!(
        row(&rated.rows, "2026-03-02").estimate.as_deref(),
        Some("5.000000")
    );
    assert_eq!(
        row(&rated.rows, "2026-03-04").estimate.as_deref(),
        Some("10.000000")
    );
    assert_eq!(rated.totals.estimate.as_deref(), Some("15.000000"));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_engine_hour_rate_prices_wall_clock_time_and_the_later_row_of_a_day_wins(pool: PgPool) {
    let mut r = Run::new("2026-03-02T10:00:00Z");
    r.duration_ms = 1_800_000; // half an hour
    r.scanned_bytes = 2_000_000_000_000; // 2 TB
    insert(&pool, &r).await;
    rate(&pool, "engine_hour", "4", day(3, 1)).await;
    // A correction for the same day: the row written later is the one in force.
    rate(&pool, "engine_hour", "6", day(3, 1)).await;
    let report = usage::report(
        &pool,
        &query(
            day(3, 2),
            day(3, 2),
            Group::Engine,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    // No scan rate: only the engine part is priced. 0.5 h x 6.
    assert_eq!(report.totals.estimate.as_deref(), Some("3.000000"));
    assert_eq!(
        row(&report.rows, "clickhouse").estimate.as_deref(),
        Some("3.000000")
    );
    rate(&pool, "scan_tb", "1.5", day(3, 2)).await;
    let both = usage::report(
        &pool,
        &query(
            day(3, 2),
            day(3, 2),
            Group::Engine,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        both.totals.estimate.as_deref(),
        Some("6.000000"),
        "3 + 2 TB x 1.5"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_empty_period_has_a_zero_total_row_and_no_rows(pool: PgPool) {
    let report = usage::report(
        &pool,
        &query(
            day(3, 1),
            day(3, 31),
            Group::Person,
            TenantScope::Tenant(tenant(TENANT_A)),
        ),
    )
    .await
    .unwrap();
    assert!(
        report.rows.is_empty(),
        "a period with no usage returns no rows"
    );
    assert_eq!(report.totals.queries, 0);
    assert_eq!(report.totals.scanned_bytes, 0);
    assert_eq!(report.totals.estimate, None);
}

// ── the index (plan 102 STOP condition) ──────────────────────────────────

/// 50,000 runs of three tenants over 120 days, then the 90-day day-sum of one
/// tenant. The plan must read `query_history_tenant_at_idx` and the call must
/// finish in under two seconds.
#[sqlx::test(migrations = "../../migrations")]
async fn the_ninety_day_sum_of_one_tenant_uses_the_tenant_time_index_and_is_fast(pool: PgPool) {
    sqlx::query(
        "INSERT INTO query_history \
         (id, sql, user_name, at, status, duration_ms, scanned_bytes, cost_units, \
          workload_class, engine, owner_id, tenant_id) \
         SELECT 'q-seed-' || n, 'SELECT 1', 'user ' || (n % 40), \
                TIMESTAMPTZ '2026-03-31T23:59:59Z' - (n * interval '207 seconds'), \
                (ARRAY['completed','completed','completed','failed','blocked','cancelled','stopped'])[1 + n % 7], \
                n % 5000, (n % 977) * 1000000, 0, 'hot-analytics', \
                (ARRAY['clickhouse','trino','duckdb'])[1 + n % 3], \
                ('00000000-0000-4000-8000-0000000000' || lpad((n % 40)::text, 2, '0'))::uuid, \
                (ARRAY['11111111-1111-4111-8111-000000000001','11111111-1111-4111-8111-000000000002','11111111-1111-4111-8111-000000000003'])[1 + n % 3]::uuid \
           FROM generate_series(1, 50000) AS g(n)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE query_history")
        .execute(&pool)
        .await
        .unwrap();

    let q = query(
        day(1, 1),
        day(3, 31),
        Group::Day,
        TenantScope::Tenant(tenant(TENANT_A)),
    );
    let started = Instant::now();
    let report = usage::report(&pool, &q).await.unwrap();
    let elapsed = started.elapsed();
    println!(
        "the 90-day day sum of {} runs took {elapsed:?}",
        report.totals.queries
    );
    assert!(report.totals.queries > 5000, "{:?}", report.totals);
    assert!(
        elapsed.as_secs_f64() < 2.0,
        "the 90-day day sum took {elapsed:?}"
    );

    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {}",
        usage::REPORT_SQL
    )))
    .bind(q.from)
    .bind(q.to)
    .bind(tenant(TENANT_A))
    .bind(false)
    .bind("day")
    .fetch_all(&pool)
    .await
    .unwrap();
    let plan = plan.join("\n");
    println!("{plan}");
    assert!(
        plan.contains("query_history_tenant_at_idx"),
        "the plan must use the tenant/time index:\n{plan}"
    );
}
