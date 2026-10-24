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
    CacheScope, Computed, Effective, Fresh, MAX_ENTRY_BYTES, ResultCache, UNWATCHABLE_BOUND, key,
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
    CacheScope::new(
        Arc::clone(cache),
        rules.0,
        if fresh { Fresh::All } else { Fresh::No },
    )
    .for_board(Some(rules.1), Vec::new())
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
    let live = board.for_chart(Some("c1"), Some(CacheRule::Fresh));
    assert_eq!(live.rule(), Effective::Off);
    assert_eq!(
        board.for_chart(Some("c1"), None).rule(),
        Effective::Minutes(30)
    );
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
    let request = CacheScope::new(Arc::clone(&cache), CacheRule::Follow, Fresh::No).for_board(
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
            Err::<Computed, _>(SpecRunFailure::Tile(
                json!({"error": crate::query_limits::STOPPED_SENTENCE}),
            ))
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

#[test]
fn the_fresh_parameter_means_all_tiles_or_the_charts_it_names() {
    assert_eq!(Fresh::parse(None), Fresh::No);
    assert_eq!(Fresh::parse(Some("")), Fresh::No);
    assert_eq!(Fresh::parse(Some("1")), Fresh::All);
    assert_eq!(Fresh::parse(Some("true")), Fresh::All);
    let some = Fresh::parse(Some("c1, c2,"));
    assert_eq!(some, Fresh::Charts(vec!["c1".to_owned(), "c2".to_owned()]));
    assert!(some.covers(Some("c2")) && !some.covers(Some("c3")) && !some.covers(None));
    assert!(Fresh::All.covers(None) && !Fresh::No.covers(Some("c1")));
}

#[tokio::test]
async fn refreshing_one_chart_recomputes_that_chart_and_leaves_the_others_remembered() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));
    let request = |fresh: Fresh| {
        CacheScope::new(Arc::clone(&cache), CacheRule::Follow, fresh).for_board(None, Vec::new())
    };
    // Real SQL: a statement that does not parse is never trusted (P-B1).
    for id in ["A", "B"] {
        request(Fresh::No)
            .for_chart(Some(id), None)
            .run(
                &ch,
                (&format!("SELECT '{id}'"), &SETTINGS),
                Some(tables()),
                counted(&runs, rows(1), true),
            )
            .await
            .unwrap();
    }
    *now.lock().unwrap() += Span::minutes(1);
    let only_a = request(Fresh::parse(Some("A")));
    let a = only_a
        .for_chart(Some("A"), None)
        .run(
            &ch,
            ("SELECT 'A'", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(2), true),
        )
        .await
        .unwrap();
    let b = only_a
        .for_chart(Some("B"), None)
        .run(
            &ch,
            ("SELECT 'B'", &SETTINGS),
            Some(tables()),
            counted(&runs, rows(2), true),
        )
        .await
        .unwrap();
    assert!(!a.remembered && b.remembered);
    assert_eq!(runs.load(Ordering::SeqCst), 3);
}

/// Reads `statement` once through a scope in the `Asia/Jakarta` report zone
/// and says whether it came from memory.
async fn read_in_jakarta(
    cache: &Arc<ResultCache>,
    ch: &Arc<ChClient>,
    runs: &Arc<AtomicUsize>,
    statement: &str,
    rules: (CacheRule, CacheRule),
) -> bool {
    scope(cache, rules, false)
        .with_zone("Asia/Jakarta")
        .run(
            ch,
            (statement, &SETTINGS),
            Some(tables()),
            counted(runs, rows(1), true),
        )
        .await
        .unwrap()
        .remembered
}

/// BI-4 B review P-B1: "Today" is `toDate(now('<zone>'))`, the same text every
/// day, and a nightly mart's fingerprint does not move at midnight.
#[tokio::test]
async fn a_statement_with_a_clock_function_is_not_served_across_midnight_in_the_report_zone() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let (now, read) = clock();
    let cache = Arc::new(ResultCache::with_clock(8, read));
    let runs = Arc::new(AtomicUsize::new(0));
    let today = "SELECT count() FROM serving.mart_sales WHERE d = toDate(now('Asia/Jakarta'))";
    let plain = "SELECT count() FROM serving.mart_sales";
    // 23:55 in Jakarta (UTC+7) on 2026-10-24.
    *now.lock().unwrap() = Utc.with_ymd_and_hms(2026, 10, 24, 16, 55, 0).unwrap();
    assert!(!read_in_jakarta(&cache, &ch, &runs, today, defaults()).await);
    assert!(!read_in_jakarta(&cache, &ch, &runs, plain, defaults()).await);
    // Still the same day, a few minutes on.
    *now.lock().unwrap() = Utc.with_ymd_and_hms(2026, 10, 24, 16, 58, 0).unwrap();
    assert!(
        read_in_jakarta(&cache, &ch, &runs, today, defaults()).await,
        "same calendar day: served"
    );
    // 00:05 on the next day in Jakarta; both are inside the ten-minute bound
    // and the fingerprint is unchanged.
    *now.lock().unwrap() = Utc.with_ymd_and_hms(2026, 10, 24, 17, 3, 0).unwrap();
    assert!(
        !read_in_jakarta(&cache, &ch, &runs, today, defaults()).await,
        "the day changed in the report zone: computed again"
    );
    assert!(
        read_in_jakarta(&cache, &ch, &runs, plain, defaults()).await,
        "a statement with no clock function is still served"
    );
}

#[tokio::test]
async fn a_clock_statement_with_no_zone_known_is_never_served() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    for _ in 0..2 {
        let answer = scope(&cache, defaults(), false)
            .run(
                &ch,
                ("SELECT rand()", &SETTINGS),
                Some(tables()),
                counted(&runs, rows(1), true),
            )
            .await
            .unwrap();
        assert!(!answer.remembered);
    }
    assert_eq!(runs.load(Ordering::SeqCst), 2);
}

/// BI-4 B review P-S1: data read through a dictionary, a `Join` table or
/// `IN <table>` is not in any `FROM`, so the fingerprint cannot watch it.
#[tokio::test]
async fn a_statement_reading_through_a_dictionary_or_in_table_is_bounded_like_an_unwatchable_one() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    for statement in [
        "SELECT dictGet('d', 'v', toUInt64(1)) FROM serving.mart_sales",
        "SELECT joinGet('j', 'v', 1) FROM serving.mart_sales",
        "SELECT count() FROM serving.mart_sales WHERE id IN other_table",
    ] {
        let (now, read) = clock();
        let cache = Arc::new(ResultCache::with_clock(8, read));
        let runs = Arc::new(AtomicUsize::new(0));
        assert!(!read_in_jakarta(&cache, &ch, &runs, statement, defaults()).await);
        *now.lock().unwrap() += Span::minutes(11);
        assert!(
            !read_in_jakarta(&cache, &ch, &runs, statement, defaults()).await,
            "past ten minutes, though the fingerprint of its FROM table is unchanged: {statement}"
        );
    }
}

#[test]
fn clock_random_and_outside_from_reads_are_told_from_plain_statements() {
    use crate::sql_rewrite::{BeyondTables, reads_beyond_tables};
    let dialect = sqlparser::dialect::ClickHouseDialect {};
    let clock = |sql| reads_beyond_tables(sql, &dialect).clock_or_random;
    assert!(clock("SELECT toDate(now('UTC'))"));
    assert!(clock("SELECT today()"));
    assert!(clock("SELECT now64(3)"));
    assert!(clock("SELECT rand() % 3"));
    assert!(clock("SELECT randCanonical()"));
    assert!(clock("SELECT generateUUIDv4()"));
    assert!(!clock("SELECT toDate(created) FROM serving.t"));
    assert_eq!(
        reads_beyond_tables("SELECT sum(x) FROM serving.t", &dialect),
        BeyondTables::default()
    );
    assert!(reads_beyond_tables("SELECT dictGetString('d','a',1)", &dialect).outside_from);
    assert!(reads_beyond_tables("SELECT 1 FROM t WHERE a IN other", &dialect).outside_from);
    // Unparseable: nothing can be said, so nothing is trusted.
    let unknown = reads_beyond_tables("SELEKT", &dialect);
    assert!(unknown.clock_or_random && unknown.outside_from);
}

/// BI-4 B review P-S2: what the leader's outcome is for every waiting tile
/// (a time limit, a cut result) is shared; only a Stop or an engine error is
/// the leader's own.
#[tokio::test]
async fn tiles_waiting_on_a_time_limit_or_a_cut_share_the_leaders_outcome() {
    let parts = Arc::new(Parts::default());
    parts.set("serving", "mart_sales", "2026-10-24 08:00:00", "10");
    let (_server, ch) = engine(&parts).await;
    let cache = Arc::new(ResultCache::new(8));
    let runs = Arc::new(AtomicUsize::new(0));
    let request = scope(&cache, defaults(), false);
    let slow_limit = {
        let runs = Arc::clone(&runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Err::<Computed, _>(SpecRunFailure::Tile(
                json!({"error": "Took longer than 60 seconds", "limit": {"kind": "time"}}),
            ))
        }
    };
    let follower = |n: &Arc<AtomicUsize>| {
        let n = Arc::clone(n);
        move || async move {
            n.fetch_add(1, Ordering::SeqCst);
            Ok(Computed {
                value: rows(9),
                complete: true,
            })
        }
    };
    let (leader, waiter) = tokio::join!(
        request.run(&ch, ("SELECT 1", &SETTINGS), Some(tables()), slow_limit),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            request
                .run(
                    &ch,
                    ("SELECT 1", &SETTINGS),
                    Some(tables()),
                    follower(&runs),
                )
                .await
        },
    );
    assert!(matches!(leader, Err(SpecRunFailure::Tile(_))));
    match waiter {
        Err(SpecRunFailure::Tile(tile)) => assert_eq!(tile["limit"]["kind"], "time"),
        other => panic!("the waiter should share the time limit, got {other:?}"),
    }
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the engine was asked once");

    // A cut result: answered to both, asked once, stored by neither.
    let runs = Arc::new(AtomicUsize::new(0));
    let cut = {
        let runs = Arc::clone(&runs);
        move || async move {
            runs.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            Ok(Computed {
                value: json!({"columns": ["n"], "rows": [], "limitHit": {"kind": "rows"}}),
                complete: false,
            })
        }
    };
    let (a, b) = tokio::join!(
        request.run(&ch, ("SELECT 2", &SETTINGS), Some(tables()), cut),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            request
                .run(
                    &ch,
                    ("SELECT 2", &SETTINGS),
                    Some(tables()),
                    follower(&runs),
                )
                .await
        },
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_eq!(a.value, b.value, "the follower got the leader's cut result");
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the engine was asked once");
    assert_eq!(cache.entry_count().await, 0);
}
