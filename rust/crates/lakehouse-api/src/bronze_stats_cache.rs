//! A time-bounded, size-bounded cache of Bronze Iceberg facts, shared
//! across requests on [`crate::state::AppState`], used by
//! `routes::catalog` to enrich `GET /api/catalog`'s Bronze rows with
//! `sizeBytes`/`freshnessLagSeconds` without a Lakekeeper round trip on
//! every warm request.
//!
//! Two independent things are cached, both on a 60 s TTL:
//! - [`CachedTableStats`] per Bronze table name (`total_bytes`,
//!   `last_updated_ms`) — `sizeBytes` may therefore be up to 60 s stale.
//! - The set of table names that actually exist in Lakekeeper's `bronze`
//!   namespace, so a warm request that has already proven a
//!   `dataset_catalog.table_name` is NOT a real Bronze Iceberg table (the
//!   common case for demo-seeded rows — see `routes::catalog`'s module
//!   doc) never repeats that listing call either.
//!
//! `std::sync::Mutex`, not `tokio::sync::Mutex`: every critical section
//! below is a synchronous `HashMap`/`HashSet` operation with no `.await`
//! inside it, so a std mutex is strictly cheaper and cannot trip
//! `clippy::await_holding_lock` by construction — there is no `.await` to
//! hold it across. A poisoned lock (a panic while held) is recovered via
//! `into_inner` rather than propagated, matching the rest of this cache's
//! philosophy: enrichment degrading to "not measured" is always
//! preferable to a panicked request.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::time::Instant;

/// How long a cached fact (per-table stats or the table-name listing) is
/// trusted before a fresh Lakekeeper call is required. `sizeBytes` and
/// `freshnessLagSeconds` may therefore be up to this stale.
const TTL: Duration = Duration::from_secs(60);

/// At most this many per-table stats entries are kept. This is a latency
/// optimization, not a correctness requirement, so a simple bound —
/// expired entries evicted first, then the oldest by `fetched_at` — is
/// enough; there is no scenario where exceeding it would serve wrong data,
/// only a scenario where an old table's stats are evicted and refetched.
const MAX_STATS_ENTRIES: usize = 1_000;

/// One Bronze Iceberg table's cached size/freshness inputs.
/// `freshnessLagSeconds` is deliberately NOT cached here — it is computed
/// at response time from `last_updated_ms` (see
/// `routes::catalog::apply_iceberg_enrichment`), so a request served from
/// this cache still reports a freshness lag current to the response, not
/// frozen at the moment the entry was fetched.
#[derive(Debug, Clone, Copy)]
pub struct CachedTableStats {
    /// `TableStats::total_bytes` from the table's current snapshot.
    pub total_bytes: Option<u64>,
    /// The current snapshot's commit timestamp, in epoch milliseconds.
    pub last_updated_ms: Option<i64>,
}

struct StatsEntry {
    stats: CachedTableStats,
    fetched_at: Instant,
}

struct TableNamesEntry {
    names: HashSet<String>,
    fetched_at: Instant,
}

fn is_expired(fetched_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(fetched_at) >= TTL
}

/// Recovers a poisoned lock rather than propagating the panic — see the
/// module doc comment on why a degraded cache always beats a panicked
/// request here.
fn recover<'a, T>(
    result: Result<MutexGuard<'a, T>, PoisonError<MutexGuard<'a, T>>>,
) -> MutexGuard<'a, T> {
    result.unwrap_or_else(PoisonError::into_inner)
}

/// The cache itself. Always present on [`crate::state::AppState`] (unlike
/// its `pg`/`auth` fields) — it needs no external dependency to construct,
/// only an in-process map, matching [`crate::gold_lock::MartLocks`]'s
/// pattern.
#[derive(Default)]
pub struct BronzeStatsCache {
    stats: Mutex<HashMap<String, StatsEntry>>,
    table_names: Mutex<Option<TableNamesEntry>>,
}

impl BronzeStatsCache {
    /// An empty cache — every lookup misses until the first
    /// `put_stats`/`put_bronze_table_names` call.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the cached stats for `table_name` if present and fetched
    /// less than [`TTL`] before `now`. `now` is a parameter, not read from
    /// the clock here, so this stays unit-testable with a fabricated
    /// `Instant` — see this module's tests.
    #[must_use]
    pub fn get_stats(&self, table_name: &str, now: Instant) -> Option<CachedTableStats> {
        let map = recover(self.stats.lock());
        let entry = map.get(table_name)?;
        if is_expired(entry.fetched_at, now) {
            None
        } else {
            Some(entry.stats)
        }
    }

    /// Inserts (or refreshes) `table_name`'s cached stats as of `now`.
    /// Expired entries are evicted first; if the map is still at
    /// [`MAX_STATS_ENTRIES`] and `table_name` is new, the single oldest
    /// entry (by `fetched_at`) is evicted to make room.
    pub fn put_stats(&self, table_name: String, stats: CachedTableStats, now: Instant) {
        let mut map = recover(self.stats.lock());
        map.retain(|_, entry| !is_expired(entry.fetched_at, now));
        if !map.contains_key(&table_name)
            && map.len() >= MAX_STATS_ENTRIES
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, entry)| entry.fetched_at)
                .map(|(key, _)| key.clone())
        {
            map.remove(&oldest);
        }
        map.insert(
            table_name,
            StatsEntry {
                stats,
                fetched_at: now,
            },
        );
    }

    /// Returns the cached set of real Bronze Iceberg table names — the
    /// result of `rest::list_table_idents` against Lakekeeper's `bronze`
    /// namespace — if fetched less than [`TTL`] before `now`.
    #[must_use]
    pub fn get_bronze_table_names(&self, now: Instant) -> Option<HashSet<String>> {
        let guard = recover(self.table_names.lock());
        let entry = guard.as_ref()?;
        if is_expired(entry.fetched_at, now) {
            None
        } else {
            Some(entry.names.clone())
        }
    }

    /// Replaces the cached Bronze table-name set, stamped `now`. A single
    /// entry, not bounded by [`MAX_STATS_ENTRIES`] — it holds one snapshot
    /// of the whole `bronze` namespace's listing, not one entry per table.
    pub fn put_bronze_table_names(&self, names: HashSet<String>, now: Instant) {
        let mut guard = recover(self.table_names.lock());
        *guard = Some(TableNamesEntry {
            names,
            fetched_at: now,
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn stats(total_bytes: u64) -> CachedTableStats {
        CachedTableStats {
            total_bytes: Some(total_bytes),
            last_updated_ms: Some(1_000),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_fresh_entry_hits_within_the_ttl() {
        let cache = BronzeStatsCache::new();
        let t0 = Instant::now();
        cache.put_stats("orders".to_owned(), stats(42), t0);

        let hit = cache.get_stats("orders", t0 + Duration::from_secs(30));
        assert!(matches!(hit, Some(s) if s.total_bytes == Some(42)));
    }

    #[tokio::test(start_paused = true)]
    async fn an_entry_misses_once_the_ttl_has_elapsed() {
        let cache = BronzeStatsCache::new();
        let t0 = Instant::now();
        cache.put_stats("orders".to_owned(), stats(42), t0);

        let miss = cache.get_stats("orders", t0 + Duration::from_secs(61));
        assert!(miss.is_none(), "an entry older than the 60s TTL must miss");
    }

    #[tokio::test(start_paused = true)]
    async fn an_unknown_table_name_misses() {
        let cache = BronzeStatsCache::new();
        assert!(cache.get_stats("unknown", Instant::now()).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_bound_evicts_the_oldest_entry_first() {
        let cache = BronzeStatsCache::new();
        let t0 = Instant::now();
        for i in 0..MAX_STATS_ENTRIES {
            cache.put_stats(
                format!("t{i}"),
                stats(1),
                t0 + Duration::from_millis(i as u64),
            );
        }
        assert!(cache.get_stats("t0", t0).is_some());

        // One more distinct key must evict "t0" (the oldest `fetched_at`),
        // not any arbitrary entry.
        cache.put_stats(
            "t_new".to_owned(),
            stats(2),
            t0 + Duration::from_millis(MAX_STATS_ENTRIES as u64),
        );
        assert!(
            cache
                .get_stats("t0", t0 + Duration::from_millis(MAX_STATS_ENTRIES as u64))
                .is_none(),
            "the oldest entry must be evicted to make room"
        );
        assert!(
            cache
                .get_stats(
                    "t_new",
                    t0 + Duration::from_millis(MAX_STATS_ENTRIES as u64)
                )
                .is_some()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_bronze_table_name_set_respects_the_same_ttl() {
        let cache = BronzeStatsCache::new();
        let t0 = Instant::now();
        let names: HashSet<String> = ["orders".to_owned(), "shipments".to_owned()]
            .into_iter()
            .collect();
        cache.put_bronze_table_names(names.clone(), t0);

        assert_eq!(
            cache.get_bronze_table_names(t0 + Duration::from_secs(59)),
            Some(names)
        );
        assert_eq!(
            cache.get_bronze_table_names(t0 + Duration::from_secs(60)),
            None
        );
    }
}
