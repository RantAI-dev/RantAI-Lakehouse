#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration as Span, TimeZone, Utc};
use lakehouse_bi::cache_rule::CacheRule;
use lakehouse_clickhouse::ChClient;
use serde_json::{Value, json};
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

use super::{
    CacheScope, Computed, Effective, MAX_ENTRY_BYTES, ResultCache, UNWATCHABLE_BOUND, key,
    max_mb_from,
};
use crate::routes::support::SpecRunFailure;
use crate::sql_rewrite::{
    NoViews, ObligationsSource, PlaceholderValues, TableObligations, enforce,
};

/// A clock the test moves.
fn clock() -> (Arc<Mutex<DateTime<Utc>>>, super::Clock) {
    let now = Arc::new(Mutex::new(
        Utc.with_ymd_and_hms(2026, 10, 24, 9, 0, 0).unwrap(),
    ));
    let read = Arc::clone(&now);
    (now, Arc::new(move || *read.lock().unwrap()))
}

/// What a fake `system.parts` says: `(database, table, modified, rows)`.
#[derive(Default)]
struct Parts {
    rows: Mutex<Vec<(String, String, String, String)>>,
    calls: AtomicUsize,
    failing: std::sync::atomic::AtomicBool,
}

impl Parts {
    fn set(&self, db: &str, table: &str, modified: &str, rows: &str) {
        let mut all = self.rows.lock().unwrap();
        all.retain(|(d, t, _, _)| !(d == db && t == table));
        all.push((db.into(), table.into(), modified.into(), rows.into()));
    }
}

async fn engine(parts: &Arc<Parts>) -> (MockServer, Arc<ChClient>) {
    let server = MockServer::start().await;
    let parts = Arc::clone(parts);
    Mock::given(method("POST"))
        .and(body_string_contains("system.parts"))
        .respond_with(move |_req: &Request| {
            parts.calls.fetch_add(1, Ordering::SeqCst);
            if parts.failing.load(Ordering::SeqCst) {
                return ResponseTemplate::new(500).set_body_string("Code: 1. planted-marker");
            }
            let data: Vec<Value> = parts
                .rows
                .lock()
                .unwrap()
                .iter()
                .map(|(d, t, m, r)| {
                    json!({"database": d, "table": t, "modified": m, "row_count": r})
                })
                .collect();
            ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": data, "rows": data.len(),
            }))
        })
        .mount(&server)
        .await;
    let ch = Arc::new(ChClient::new(
        server.uri(),
        "default".to_owned(),
        String::new(),
    ));
    (server, ch)
}

fn scope(cache: &Arc<ResultCache>, rules: (CacheRule, CacheRule), fresh: bool) -> CacheScope {
    CacheScope::new(Arc::clone(cache), rules.0, fresh).for_board(Some(rules.1), Vec::new())
}

const SETTINGS: [(&str, u64); 2] = [("max_execution_time", 60_000), ("max_result_rows", 2001)];

/// A computation that counts its runs and answers `value`.
fn counted(
    runs: &Arc<AtomicUsize>,
    value: Value,
    complete: bool,
) -> impl FnOnce() -> std::future::Ready<Result<Computed, SpecRunFailure>> + Send {
    let runs = Arc::clone(runs);
    move || {
        runs.fetch_add(1, Ordering::SeqCst);
        std::future::ready(Ok(Computed { value, complete }))
    }
}

fn rows(n: i64) -> Value {
    json!({"columns": ["n"], "rows": [{"n": n}]})
}

fn tables() -> Vec<String> {
    vec!["serving.mart_sales".to_owned()]
}

fn defaults() -> (CacheRule, CacheRule) {
    (CacheRule::Follow, CacheRule::Follow)
}

#[test]
fn equal_inputs_give_one_key_and_any_difference_gives_another() {
    let base = key("clickhouse", "SELECT 1", &SETTINGS);
    assert_eq!(base, key("clickhouse", "SELECT 1", &SETTINGS));
    assert_ne!(base, key("clickhouse", "SELECT 2", &SETTINGS));
    assert_ne!(base, key("trino", "SELECT 1", &SETTINGS));
    // The BI-5 limits: a result computed under one limit is never served under
    // another.
    assert_ne!(
        base,
        key(
            "clickhouse",
            "SELECT 1",
            &[("max_execution_time", 30_000), ("max_result_rows", 2001)]
        )
    );
    assert_ne!(
        base,
        key(
            "clickhouse",
            "SELECT 1",
            &[("max_execution_time", 60_000), ("max_result_rows", 101)]
        )
    );
    // Length prefixes: moving a byte across two fields changes the key.
    assert_ne!(key("ab", "c", &[]), key("a", "bc", &[]));
}

/// Obligations by role: `Analyst` has `email` masked, `Rep` sees only their
/// own rows, `Admin` has none.
struct ByRole;

impl ObligationsSource for ByRole {
    fn obligations_for(&self, table: &str, roles: &[String]) -> Option<TableObligations> {
        if table != "silver.customers" {
            return None;
        }
        let has = |role: &str| roles.iter().any(|r| r == role);
        let real = Some(vec![
            "id".to_owned(),
            "email".to_owned(),
            "owner".to_owned(),
        ]);
        if has("Analyst") {
            Some(TableObligations {
                mask: vec!["email".to_owned()],
                row_filter: None,
                real_columns: real,
            })
        } else if has("Rep") {
            Some(TableObligations {
                mask: Vec::new(),
                row_filter: Some("owner = __principal_id__".to_owned()),
                real_columns: real,
            })
        } else {
            None
        }
    }

    fn has_any_obligation(&self, roles: &[String]) -> bool {
        roles.iter().any(|r| r == "Analyst" || r == "Rep")
    }
}

fn rewritten(roles: &[&str], principal: &str) -> String {
    let roles: Vec<String> = roles.iter().map(|r| (*r).to_owned()).collect();
    enforce(
        "SELECT id, email FROM silver.customers",
        &sqlparser::dialect::ClickHouseDialect {},
        &roles,
        &PlaceholderValues {
            principal_id: Some(principal.to_owned()),
            principal_tenant_ids: Vec::new(),
        },
        &ByRole,
        &NoViews,
    )
    .unwrap()
}

/// Feature decision 4 and plan D4: the key is made of the statement after the
/// rewrite, and the rewrite puts the person's masks and own id into it.
#[test]
fn two_role_sets_with_different_masks_and_two_principals_with_a_row_filter_never_share_a_key() {
    let k = |text: &str| key("clickhouse", text, &SETTINGS);
    let (masked, unmasked) = (rewritten(&["Analyst"], "u-1"), rewritten(&["Admin"], "u-1"));
    assert_ne!(masked, unmasked, "the mask is in the statement");
    assert_ne!(k(&masked), k(&unmasked));
    let (first, second) = (rewritten(&["Rep"], "u-1"), rewritten(&["Rep"], "u-2"));
    assert!(
        first.contains("u-1") && second.contains("u-2"),
        "{first} / {second}"
    );
    assert_ne!(k(&first), k(&second));
    // The same rules and the same person: one statement, one key, so two equal
    // viewers may share a result.
    assert_eq!(k(&rewritten(&["Rep"], "u-1")), k(&first));
    assert_eq!(
        k(&rewritten(&["Analyst"], "u-1")),
        k(&rewritten(&["Analyst"], "u-2"))
    );
}

#[test]
fn the_narrowest_rule_that_is_set_wins() {
    use CacheRule::{Follow, Fresh, Minutes};
    assert_eq!(
        Effective::resolve(Follow, Follow, Follow),
        Effective::Default
    );
    assert_eq!(Effective::resolve(Follow, Follow, Fresh), Effective::Off);
    assert_eq!(
        Effective::resolve(Follow, Minutes(5), Fresh),
        Effective::Minutes(5)
    );
    assert_eq!(
        Effective::resolve(Fresh, Minutes(5), Minutes(9)),
        Effective::Off
    );
    assert_eq!(
        Effective::resolve(Minutes(2), Fresh, Fresh),
        Effective::Minutes(2)
    );
}

#[test]
fn the_memory_setting_is_clamped_and_zero_switches_remembering_off() {
    assert_eq!(max_mb_from(None), 256);
    assert_eq!(max_mb_from(Some("")), 256);
    assert_eq!(max_mb_from(Some("nope")), 256);
    assert_eq!(max_mb_from(Some("0")), 0);
    assert_eq!(max_mb_from(Some(" 64 ")), 64);
    assert_eq!(max_mb_from(Some("99999")), 4096);
    assert!(!ResultCache::new(0).enabled());
    assert!(ResultCache::new(1).enabled());
}

#[tokio::test]
async fn a_second_read_is_remembered_with_the_first_reads_computed_at() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));

    let first = scope(&cache, defaults(), false)
        .run(
            &ch,
            ("SELECT 1", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(1), true),
        )
        .await
        .unwrap();
    assert!(!first.remembered);
    *now.lock().unwrap() += Span::minutes(3);
    let second = scope(&cache, defaults(), false)
        .run(
            &ch,
            ("SELECT 1", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(2), true),
        )
        .await
        .unwrap();
    assert!(second.remembered);
    assert_eq!(
        second.value,
        rows(1),
        "the first answer, not a recomputation"
    );
    assert_eq!(second.computed_at, first.computed_at);
    assert_eq!(runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fresh_skips_the_lookup_and_replaces_the_entry() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));
    let run = |fresh: bool, n: i64| {
        let (cache, ch, runs) = (Arc::clone(&cache), Arc::clone(&ch), Arc::clone(&runs));
        async move {
            scope(&cache, defaults(), fresh)
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    counted(&runs, rows(n), true),
                )
                .await
                .unwrap()
        }
    };
    let first = run(false, 1).await;
    *now.lock().unwrap() += Span::minutes(1);
    let forced = run(true, 2).await;
    assert!(!forced.remembered);
    assert_eq!(forced.value, rows(2));
    assert!(forced.computed_at > first.computed_at, "a new computedAt");
    *now.lock().unwrap() += Span::minutes(1);
    let after = run(false, 3).await;
    assert!(after.remembered);
    assert_eq!(after.value, rows(2), "the forced result replaced the entry");
}

#[tokio::test]
async fn a_result_over_eight_megabytes_a_cut_result_and_an_error_are_never_stored() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(64));
    let runs = Arc::new(AtomicUsize::new(0));

    let big = json!({"columns": ["s"], "rows": [{"s": "x".repeat(MAX_ENTRY_BYTES + 1)}]});
    let answered = scope(&cache, defaults(), false)
        .run(
            &ch,
            ("big", &SETTINGS),
            Some(tables()),
            counted(&runs, big.clone(), true),
        )
        .await
        .unwrap();
    assert_eq!(answered.value, big, "answered in full, only not remembered");
    scope(&cache, defaults(), false)
        .run(
            &ch,
            ("big", &SETTINGS),
            Some(tables()),
            counted(&runs, big, true),
        )
        .await
        .unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 2);

    for _ in 0..2 {
        let cut = json!({"columns": ["n"], "rows": [], "limitHit": {"kind": "rows"}});
        scope(&cache, defaults(), false)
            .run(
                &ch,
                ("cut", &SETTINGS),
                Some(tables()),
                counted(&runs, cut, false),
            )
            .await
            .unwrap();
    }
    assert_eq!(runs.load(Ordering::SeqCst), 4, "a cut result runs again");

    for _ in 0..2 {
        let failed = scope(&cache, defaults(), false)
            .run(&ch, ("err", &SETTINGS), Some(tables()), || {
                runs.fetch_add(1, Ordering::SeqCst);
                std::future::ready(Err::<Computed, _>(SpecRunFailure::Tile(
                    json!({"error": "stopped"}),
                )))
            })
            .await;
        assert!(matches!(failed, Err(SpecRunFailure::Tile(_))));
    }
    assert_eq!(runs.load(Ordering::SeqCst), 6, "a failed tile runs again");
    assert_eq!(cache.entry_count().await, 0);
}

#[tokio::test]
async fn the_limits_are_in_the_key() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let other = [("max_execution_time", 30_000), ("max_result_rows", 2001)];
    for settings in [&SETTINGS, &other, &SETTINGS, &other] {
        scope(&cache, defaults(), false)
            .run(
                &ch,
                ("SELECT 1", settings),
                Some(tables()),
                counted(&runs, rows(1), true),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "one run per limit, then both remembered"
    );
}

#[tokio::test]
async fn a_changed_fingerprint_misses_and_the_new_result_is_remembered() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let read = |n: i64| {
        let (cache, ch, runs) = (Arc::clone(&cache), Arc::clone(&ch), Arc::clone(&runs));
        async move {
            scope(&cache, defaults(), false)
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    counted(&runs, rows(n), true),
                )
                .await
                .unwrap()
        }
    };
    assert!(!read(1).await.remembered);
    assert!(read(2).await.remembered, "nothing changed");
    // A Gold build wrote a new part.
    parts.set("serving", "mart_sales", "2026-10-24 09:30:00", "11");
    let changed = read(3).await;
    assert!(!changed.remembered);
    assert_eq!(changed.value, rows(3));
    assert!(read(4).await.remembered);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_table_the_engine_cannot_watch_is_remembered_for_ten_minutes_by_default() {
    let parts = Arc::new(Parts::default());
    // Nothing listed: a view or an Iceberg table engine.
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));
    let read_once = |rules: (CacheRule, CacheRule)| {
        let (cache, ch, runs) = (Arc::clone(&cache), Arc::clone(&ch), Arc::clone(&runs));
        async move {
            scope(&cache, rules, false)
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    counted(&runs, rows(1), true),
                )
                .await
                .unwrap()
        }
    };
    assert!(!read_once(defaults()).await.remembered);
    *now.lock().unwrap() += UNWATCHABLE_BOUND
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap();
    assert!(
        read_once(defaults()).await.remembered,
        "just inside the bound"
    );
    *now.lock().unwrap() += Span::seconds(2).to_std().unwrap();
    assert!(
        !read_once(defaults()).await.remembered,
        "past ten minutes it is computed again"
    );
    // A person who set 60 minutes asked for 60.
    *now.lock().unwrap() += Span::minutes(20).to_std().unwrap();
    let hour = (CacheRule::Follow, CacheRule::Minutes(60));
    assert!(read_once(hour).await.remembered);
}

#[tokio::test]
async fn minutes_bound_the_age_even_when_nothing_changed() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));
    let five = (CacheRule::Follow, CacheRule::Minutes(5));
    let read_once = || {
        let (cache, ch, runs) = (Arc::clone(&cache), Arc::clone(&ch), Arc::clone(&runs));
        async move {
            scope(&cache, five, false)
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    counted(&runs, rows(1), true),
                )
                .await
                .unwrap()
        }
    };
    assert!(!read_once().await.remembered);
    *now.lock().unwrap() += Span::minutes(4);
    assert!(read_once().await.remembered);
    *now.lock().unwrap() += Span::minutes(2);
    assert!(
        !read_once().await.remembered,
        "six minutes old under a five minute rule"
    );
}

#[tokio::test]
async fn always_fresh_and_a_switched_off_store_never_store() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let runs = Arc::new(AtomicUsize::new(0));
    let on = Arc::new(ResultCache::new(8));
    let off = Arc::new(ResultCache::new(0));
    for (cache, rules) in [
        (&on, (CacheRule::Follow, CacheRule::Fresh)),
        (&on, (CacheRule::Fresh, CacheRule::Follow)),
        (&off, defaults()),
    ] {
        for _ in 0..2 {
            scope(cache, rules, false)
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    counted(&runs, rows(1), true),
                )
                .await
                .unwrap();
        }
    }
    assert_eq!(runs.load(Ordering::SeqCst), 6);
    assert_eq!(on.entry_count().await, 0);
}

#[tokio::test]
async fn a_chart_rule_narrows_the_dashboards() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let board = scope(&cache, (CacheRule::Follow, CacheRule::Minutes(30)), false);
    let live = board.for_chart(Some(CacheRule::Fresh));
    assert_eq!(live.rule(), Effective::Off);
    assert_eq!(board.for_chart(None).rule(), Effective::Minutes(30));
    for _ in 0..2 {
        live.run(
            &ch,
            ("SELECT 1", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(1), true),
        )
        .await
        .unwrap();
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_fingerprint_that_cannot_be_read_computes_fresh_and_stores_nothing() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    parts.failing.store(true, Ordering::SeqCst);
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 {
        let answer = scope(&cache, defaults(), false)
            .run(
                &ch,
                ("SELECT 1", &SETTINGS),
                Some(tables()),
                counted(&runs, rows(1), true),
            )
            .await
            .unwrap();
        assert!(!answer.remembered);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(
        cache.entry_count().await,
        0,
        "nothing stored without a fingerprint"
    );
    // And it recovers: once the engine answers, results are remembered again.
    parts.failing.store(false, Ordering::SeqCst);
    scope(&cache, defaults(), false)
        .run(
            &ch,
            ("SELECT 1", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(1), true),
        )
        .await
        .unwrap();
    assert_eq!(cache.entry_count().await, 1);
}

#[tokio::test]
async fn a_statement_whose_tables_cannot_be_named_is_not_stored() {
    let parts = Arc::new(Parts::default());
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 {
        scope(&cache, defaults(), false)
            .run(
                &ch,
                ("SELECT 1", &SETTINGS),
                None,
                counted(&runs, rows(1), true),
            )
            .await
            .unwrap();
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(parts.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn two_tiles_with_one_statement_run_one_query_and_one_request_reads_fingerprints_once() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    parts.set("serving", "mart_other", "2026-10-24 08:00:00", "5");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let request = CacheScope::new(Arc::clone(&cache), CacheRule::Follow, false).for_board(
        None,
        vec![
            "serving.mart_sales".to_owned(),
            "serving.mart_other".to_owned(),
        ],
    );
    let slow = |runs: &Arc<AtomicUsize>| {
        let runs = Arc::clone(runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Ok(Computed {
                value: rows(1),
                complete: true,
            })
        }
    };
    let (a, b, c) = tokio::join!(
        request.run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), slow(&runs)),
        request.run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), slow(&runs)),
        request.run(
            &ch,
            ("SELECT 2", &SETTINGS),
            Some(vec!["serving.mart_other".to_owned()]),
            slow(&runs)
        ),
    );
    assert_eq!(a.unwrap().value, rows(1));
    assert_eq!(b.unwrap().value, rows(1));
    c.unwrap();
    assert_eq!(
        runs.load(Ordering::SeqCst),
        2,
        "SELECT 1 once, SELECT 2 once"
    );
    assert_eq!(
        parts.calls.load(Ordering::SeqCst),
        1,
        "the candidates were fingerprinted together, once"
    );
}

#[tokio::test]
async fn a_tile_waiting_on_a_failed_tile_asks_the_engine_itself() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let request = scope(&cache, defaults(), false);
    let failing = {
        let runs = Arc::clone(&runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Err::<Computed, _>(SpecRunFailure::Tile(json!({"error": "stopped"})))
        }
    };
    let ok = {
        let runs = Arc::clone(&runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            Ok(Computed {
                value: rows(7),
                complete: true,
            })
        }
    };
    let (leader, waiter) = tokio::join!(
        request.run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), failing),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            request
                .run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), ok)
                .await
        },
    );
    assert!(matches!(leader, Err(SpecRunFailure::Tile(_))));
    assert_eq!(
        waiter.unwrap().value,
        rows(7),
        "the waiter was not failed by the leader"
    );
}

/// A run that is cancelled (Stop, a closed request) leaves no half-written
/// entry: the next read computes.
#[tokio::test]
async fn a_cancelled_computation_leaves_nothing_behind() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let never = {
        let runs = Arc::clone(&runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            std::future::pending::<Result<Computed, SpecRunFailure>>().await
        }
    };
    let cut = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        scope(&cache, defaults(), false).run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), never),
    )
    .await;
    assert!(cut.is_err(), "the request was cancelled");
    assert_eq!(cache.entry_count().await, 0);
    let answer = scope(&cache, defaults(), false)
        .run(
            &ch,
            ("SELECT 1", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(1), true),
        )
        .await
        .unwrap();
    assert!(!answer.remembered);
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}
