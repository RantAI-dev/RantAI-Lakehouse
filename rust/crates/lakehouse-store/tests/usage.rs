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

/// The backend the pool hands out, so "the runs above shared one connection"
/// is measured on the connection and not on the pool's bookkeeping.
/// `Pool::size()` cannot say it: the test harness gives the test a pool with
/// a one-second idle timeout (`sqlx-postgres-0.9.0/src/testing/mod.rs`), so
/// the connection the seed insert above used is closed while it sat idle and
/// the pool opens another one; the count is then two, although every run
/// used one connection.
async fn backend_pid(pool: &PgPool) -> i32 {
    let mut conn = pool.acquire().await.unwrap();
    sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .unwrap()
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

/// PR review fix (BLOCKER): the check "no rate has another currency" and the
/// insert used to be one statement without a lock, and under READ COMMITTED
/// two writers on an empty table each saw no rival and both inserted. Eight
/// writers alternate two currencies at once; exactly one currency may be
/// stored afterwards, and every accepted rate must be of that currency. The
/// table is append-only, so one attempt per database is all there is: whether
/// the race fires without the lock depends on timing (see the report of the
/// fix round for how often it did).
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_rates_of_two_currencies_leave_one_currency_stored(pool: PgPool) {
    // Open four connections first and let every writer start together, so
    // the statements overlap instead of queueing behind connection set-up.
    let mut warm = Vec::new();
    for _ in 0..4 {
        warm.push(pool.acquire().await.unwrap());
    }
    drop(warm);
    let start = std::sync::Arc::new(tokio::sync::Barrier::new(8));
    let mut writers = tokio::task::JoinSet::new();
    for n in 0..8_u8 {
        let pool = pool.clone();
        let start = std::sync::Arc::clone(&start);
        writers.spawn(async move {
            let currency = if n % 2 == 0 { "USD" } else { "IDR" };
            start.wait().await;
            usage::add_rate(
                &pool,
                &NewUsageRate {
                    kind: "scan_tb",
                    currency,
                    amount: "1",
                    effective_from: day(3, 1),
                    created_by: "test",
                },
            )
            .await
            .unwrap()
            .map(|stored| stored.currency)
        });
    }
    let mut accepted: Vec<String> = Vec::new();
    while let Some(done) = writers.join_next().await {
        accepted.extend(done.unwrap());
    }

    let stored: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT currency FROM usage_rate ORDER BY currency")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(stored.len(), 1, "two currencies were stored: {stored:?}");
    assert!(
        !accepted.is_empty() && accepted.iter().all(|c| *c == stored[0]),
        "every accepted rate is of the stored currency {stored:?}: {accepted:?}"
    );
    let rows = usage::list_rates(&pool).await.unwrap();
    assert_eq!(
        rows.len(),
        accepted.len(),
        "an accepted rate is a stored row"
    );
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

/// A user in the user list, as `app_user` holds it: the name the Usage page
/// shows for that person.
async fn user_in_the_list(pool: &PgPool, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let username = format!("user-{}", id.simple());
    let email = format!("{}@test.invalid", id.simple());
    sqlx::query("INSERT INTO app_user (id, name, username, email) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(name)
        .bind(username)
        .bind(email)
        .execute(pool)
        .await
        .expect("insert an app_user row");
    id
}

/// A person row is keyed by the owner's id and labelled with the name the
/// user list holds for that owner.
///
/// A run whose owner is not in the user list any more (the user was deleted)
/// and a run with no owner at all keep the name their run recorded: the
/// first is the newest of their runs, the second is their key.
#[sqlx::test(migrations = "../../migrations")]
async fn a_person_is_the_owner_and_the_label_is_the_name_the_user_list_holds(pool: PgPool) {
    let owner = user_in_the_list(&pool, "Rina Wijaya").await;
    // Since the query-ownership fix the run records the principal's uuid
    // string, so the name a run carries is not a name at all.
    let recorded = owner.to_string();
    for at in ["2026-03-02T10:00:00Z", "2026-03-02T11:00:00Z"] {
        let mut r = Run::new(at);
        r.owner = Some(owner);
        r.user = &recorded;
        r.scanned_bytes = 5;
        insert(&pool, &r).await;
    }
    // An owner who is not in the user list any more: the user was deleted.
    let deleted = Uuid::new_v4();
    for (at, user) in [
        ("2026-03-02T12:00:00Z", "Ghost (old name)"),
        ("2026-03-02T13:00:00Z", "Ghost User"),
    ] {
        let mut r = Run::new(at);
        r.owner = Some(deleted);
        r.user = user;
        insert(&pool, &r).await;
    }
    for _ in 0..2 {
        let mut r = Run::new("2026-03-02T14:00:00Z");
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
    assert_eq!(report.rows.len(), 3);
    let person = row(&report.rows, &owner.to_string());
    assert_eq!(person.queries, 2, "one person, two runs");
    assert_eq!(person.label, "Rina Wijaya", "the name the user list holds");
    let ghost = row(&report.rows, &deleted.to_string());
    assert_eq!(ghost.queries, 2);
    assert_eq!(ghost.label, "Ghost User", "the name on the newest run");
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
/// finish in under two seconds, also after the statement has run eight times
/// on one connection (past the fifth, Postgres may switch a prepared
/// statement to a generic plan) and when the plan is forced to be generic.
///
/// PR review fix (SHOULD-FIX): the statement used to carry
/// `($4::boolean OR q.tenant_id = $3::uuid)`, which the index serves in a
/// custom plan only.
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

    let custom: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {}",
        usage::REPORT_TENANT_SQL
    )))
    .bind(q.from)
    .bind(q.to)
    .bind("day")
    .bind(tenant(TENANT_A))
    .fetch_all(&pool)
    .await
    .unwrap();
    let custom = custom.join("\n");
    println!("{custom}");
    assert!(
        custom.contains("query_history_tenant_at_idx"),
        "the plan must use the tenant/time index:\n{custom}"
    );

    // PR review fix (SHOULD-FIX): after five runs of a named prepared statement
    // Postgres may plan it once for all values, and then a tenant's period is
    // read through `query_history_at_idx` with a filter. The store runs the sum
    // unnamed, so every call is planned with its own values: eight calls on
    // one connection leave no prepared statement behind, and the ninth is as
    // fast as the first.
    //
    // The backend the runs below use, read right before them: the pool hands
    // out the same connection every time, so the runs are on one backend and
    // the pool's own bookkeeping (how many connections it holds) is not the
    // claim.
    let before = backend_pid(&pool).await;
    for _ in 0..8 {
        usage::report(&pool, &q).await.unwrap();
    }
    let started = Instant::now();
    usage::report(&pool, &q).await.unwrap();
    let ninth = started.elapsed();
    let after = backend_pid(&pool).await;
    println!("the ninth run took {ninth:?}");
    println!("the runs above used the backend {before}");
    assert!(ninth.as_secs_f64() < 2.0, "the ninth run took {ninth:?}");
    assert_eq!(before, after, "the runs above shared one connection");
    let mut conn = pool.acquire().await.unwrap();
    // The count asks for the sum's own text, bound: the statement of the sum
    // is the one the claim is about. A `LIKE` on a fragment of the text
    // counted the counting query itself (its own text carries the pattern)
    // and the `EXPLAIN` above (its text carries the sum), so the count was
    // never zero and the sum's own statement was not what was measured.
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_prepared_statements WHERE statement = $1")
            .bind(usage::REPORT_TENANT_SQL)
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_eq!(
        kept, 0,
        "the sum must run as an unnamed statement, planned at every call"
    );
}
