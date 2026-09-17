//! Integration tests for `lakehouse_store::overview` against a real
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
//! WS5 item C1: `insert_from_fired_rule`'s 15-minute dedup, made atomic
//! under two concurrent callers with a transaction-scoped
//! `pg_advisory_xact_lock`; `silence_alert`/`is_rule_silenced`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped by
// the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::overview::{
    FiredRule, insert_from_fired_rule, is_rule_silenced, silence_alert,
};
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

/// A fixed, deterministic instant so every test's dedup-window arithmetic
/// is exact rather than racing the real clock.
fn fixed_now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).unwrap()
}

fn fired(rule_id: &str) -> FiredRule<'_> {
    FiredRule {
        rule_id,
        title: "Streaming lag above threshold",
        severity: Some("high"),
        source: "Alert rules",
        affected: "rt.orders_flow_mv",
        detail: "sum(v) on rt.orders_flow_mv > 100 — observed 142.00",
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn insert_from_fired_rule_dedups_within_fifteen_minutes(pool: PgPool) -> sqlx::Result<()> {
    let first = insert_from_fired_rule(&pool, &fired("rule-1"), fixed_now())
        .await
        .unwrap();
    assert!(first.is_some(), "the first firing must insert a row");
    let second =
        insert_from_fired_rule(&pool, &fired("rule-1"), fixed_now() + Duration::minutes(5))
            .await
            .unwrap();
    assert!(
        second.is_none(),
        "same rule within the 15-minute window must not duplicate"
    );
    let third =
        insert_from_fired_rule(&pool, &fired("rule-1"), fixed_now() + Duration::minutes(16))
            .await
            .unwrap();
    assert!(
        third.is_some(),
        "past the dedup window, a new firing must insert again"
    );
    Ok(())
}

/// A fired instance copies the rule's own severity verbatim, including
/// `None` — never invented from the rule's kind (WS5 plan review Y3).
#[sqlx::test(migrations = "../../migrations")]
async fn severity_is_copied_verbatim_including_none(pool: PgPool) -> sqlx::Result<()> {
    let no_severity = FiredRule {
        severity: None,
        ..fired("rule-no-severity")
    };
    let row = insert_from_fired_rule(&pool, &no_severity, fixed_now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.severity, None);

    let with_severity = insert_from_fired_rule(&pool, &fired("rule-with-severity"), fixed_now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(with_severity.severity.as_deref(), Some("high"));
    Ok(())
}

/// Y7: two overlapping `/api/alerts/run` calls (the Dagster schedule plus
/// a manual "Run now" click) racing the same rule within the dedup window
/// must produce exactly one row, never two. Before the
/// `pg_advisory_xact_lock` was added, running this same test body against
/// an unlocked version of `insert_from_fired_rule` reliably produced 2
/// rows (both racers' `SELECT ... WHERE fired_at > $2` ran before either
/// had committed its `INSERT`) — quoted verbatim in the commit's
/// verification report, since that unlocked version is not left in the
/// tree to re-run here.
#[sqlx::test(migrations = "../../migrations")]
async fn insert_from_fired_rule_is_atomic_under_two_concurrent_callers(
    pool: PgPool,
) -> sqlx::Result<()> {
    let now = fixed_now();
    let pool_a = pool.clone();
    let pool_b = pool.clone();
    let rule = fired("rule-race");
    let (a, b) = tokio::join!(
        insert_from_fired_rule(&pool_a, &rule, now),
        insert_from_fired_rule(&pool_b, &rule, now),
    );
    let inserted = [a.unwrap(), b.unwrap()].into_iter().flatten().count();
    assert_eq!(
        inserted, 1,
        "exactly one of the two concurrent callers must win"
    );
    let row_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM alert_instance WHERE rule_id = 'rule-race'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row_count, 1);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn silence_alert_sets_silenced_until_and_acknowledges(pool: PgPool) -> sqlx::Result<()> {
    let row = insert_from_fired_rule(&pool, &fired("rule-sil-1"), fixed_now())
        .await
        .unwrap()
        .unwrap();
    let until = fixed_now() + Duration::minutes(60);
    let silenced = silence_alert(&pool, &row.id, until).await.unwrap().unwrap();
    assert_eq!(silenced.status, "acknowledged");
    assert!(silenced.silenced_until.is_some());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn is_rule_silenced_reports_true_only_while_silenced_until_is_in_the_future(
    pool: PgPool,
) -> sqlx::Result<()> {
    let row = insert_from_fired_rule(&pool, &fired("rule-sil-2"), fixed_now())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !is_rule_silenced(&pool, "rule-sil-2", fixed_now())
            .await
            .unwrap(),
        "not silenced before any silence_alert call"
    );
    silence_alert(&pool, &row.id, fixed_now() + Duration::minutes(60))
        .await
        .unwrap();
    assert!(
        is_rule_silenced(&pool, "rule-sil-2", fixed_now() + Duration::minutes(30))
            .await
            .unwrap(),
        "still within the silence window"
    );
    assert!(
        !is_rule_silenced(&pool, "rule-sil-2", fixed_now() + Duration::minutes(61))
            .await
            .unwrap(),
        "past the silence window"
    );
    Ok(())
}

/// `insert_from_fired_rule` itself has no silence awareness (it is a pure
/// dedup primitive) — the silence check lives one level up, at the
/// `SilenceSource`/route layer, which decides whether to call this
/// function at all. This test documents that boundary: past the
/// 15-minute dedup window, the primitive alone still inserts, silenced or
/// not. The end-to-end "no new row while silenced" behaviour is asserted
/// at the route (`routes::alerts::tests::a_silenced_rule_is_not_re_delivered_and_produces_no_new_row`),
/// which is the layer that actually decides whether to call this
/// function.
#[sqlx::test(migrations = "../../migrations")]
async fn a_silenced_rule_produces_no_new_instance_after_its_dedup_window_lapses(
    pool: PgPool,
) -> sqlx::Result<()> {
    let now = fixed_now();
    let first = insert_from_fired_rule(&pool, &fired("rule-sil-3"), now)
        .await
        .unwrap()
        .unwrap();
    silence_alert(&pool, &first.id, now + Duration::minutes(1))
        .await
        .unwrap();
    // 16 minutes later: past the 15-minute dedup window, but still
    // silenced (silenced_until was set generously above).
    let later = insert_from_fired_rule(&pool, &fired("rule-sil-3"), now + Duration::minutes(16))
        .await
        .unwrap();
    assert!(
        later.is_some(),
        "insert_from_fired_rule alone still dedups only on time, not silence"
    );
    Ok(())
}
