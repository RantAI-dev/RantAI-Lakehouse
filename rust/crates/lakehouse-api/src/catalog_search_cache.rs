//! The search copy of the catalog (`DATA-11` D2): one assembled catalog,
//! its columns and its use counts, kept in [`crate::state::AppState`] so a
//! search reads memory instead of rebuilding the catalog (six `ClickHouse`
//! queries and four enrichment steps) on every keystroke.
//!
//! # What may be in it
//!
//! The copy is shared by every caller, so nothing per caller may be in it.
//! The enrichment steps that fill it take no `Principal`; the tenant check
//! (`catalog_tenant_refusal`) stays per request, before the copy is read.
//!
//! # Freshness
//!
//! The copy is rebuilt when it is older than the age given to
//! [`SearchSnapshotCache::get_or_build`] (`SEARCH_SNAPSHOT_TTL`, thirty
//! seconds), but the person searching does not wait for that rebuild
//! (`DATA-38` D1):
//!
//! | Age of the copy | What a search gets |
//! | --- | --- |
//! | younger than the TTL | the copy, as it is |
//! | from the TTL to [`MAX_STALE`] | the copy at once, and ONE background rebuild behind it |
//! | [`MAX_STALE`] or older, or no copy at all | a copy built inline, single-flight, as before |
//!
//! The background rebuild is the point of `DATA-38`: on a catalog of 10,000
//! tables it takes about five seconds, and before this change every
//! thirtieth second somebody waited for it. A rebuild that fails is
//! logged, the old copy stays, and the next search after the TTL starts
//! another one. No copy older than [`MAX_STALE`] is ever served.
//!
//! A successful annotation write calls [`SearchSnapshotCache::invalidate`],
//! so a console edit shows at once: the copy is dropped and the next
//! search builds it again, inline (`DATA-38` D2).
//!
//! The clock is a parameter, as in [`crate::bronze_stats_cache`], so the
//! age rules are unit-tested without sleeping.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;

/// How old the copy may be when it is still served (`DATA-38` D1).
///
/// A copy older than this is not served at all: the read that finds it
/// builds a new one inline, so the caller waits for fresh data or gets an
/// error, but never an answer from a copy that is minutes old. Five
/// minutes is long enough to ride out a failed rebuild or a slow
/// `ClickHouse`, and short enough that a copy that cannot be rebuilt does
/// not hide the catalog for long.
pub const MAX_STALE: Duration = Duration::from_mins(5);

/// The assembled catalog as search needs it. Built once per rebuild.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogSearchSnapshot {
    /// The asset rows after the same enrichment steps `GET /api/catalog`
    /// runs.
    pub assets: Vec<Value>,
    /// The namespace summaries of the same build.
    pub namespaces: Value,
    /// Column names and descriptions per asset id.
    pub columns: HashMap<String, Vec<(String, String)>>,
    /// Queries per asset id over the use window.
    pub usage: HashMap<String, u32>,
    /// True when a column query hit its row limit: column search covers
    /// only part of the catalog.
    pub column_search_partial: bool,
}

struct Entry {
    snapshot: Arc<CatalogSearchSnapshot>,
    built_at: Instant,
}

/// What the cache holds: the copy, whether a background rebuild runs, and
/// the generation that says which copy a rebuild belongs to.
#[derive(Default)]
struct Slot {
    /// The copy, when there is one.
    entry: Option<Entry>,
    /// True while a background rebuild runs (`DATA-38` D1). One at a
    /// time: a second read of an expired copy finds the flag set and
    /// starts no second rebuild.
    rebuilding: bool,
    /// Bumped when the copy is dropped or replaced. A background rebuild
    /// that started before the bump does not put its copy back: it may
    /// have read the catalog before the write that dropped the copy, and
    /// it must not outlive that write (`DATA-11` D2).
    generation: u64,
}

/// The cache. Always present on [`crate::state::AppState`]; it needs no
/// external dependency.
///
/// A `tokio` mutex, not a `std` one: the lock is held across the inline
/// rebuild's `.await`s on purpose, which is what makes that rebuild
/// single-flight. The background rebuild holds the lock only for the
/// moment it reads the copy and the moment it puts the new one in, so a
/// search never waits for it (`DATA-38` D1).
#[derive(Default)]
pub struct SearchSnapshotCache {
    slot: Mutex<Slot>,
}

impl SearchSnapshotCache {
    /// An empty cache: the first search builds the copy.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The copy, rebuilt with `build` when there is none or it is `ttl` old
    /// or older at `now`.
    ///
    /// A copy that is older than `ttl` but younger than [`MAX_STALE`] is
    /// served at once, and one background task rebuilds it (`DATA-38`
    /// D1): the person searching does not wait for the rebuild. A copy
    /// that is [`MAX_STALE`] or older, or no copy at all, is built inline,
    /// single-flight, as before.
    ///
    /// `build` is moved into that background task, so it must own what it
    /// needs (`'static`): the caller hands it a clone of the state.
    ///
    /// # Errors
    ///
    /// Returns `build`'s error unchanged when the copy is built inline.
    /// A background rebuild that fails is logged and keeps the old copy;
    /// its error reaches nobody, because the search it answered already
    /// has an answer.
    pub async fn get_or_build<F, Fut, E>(
        self: &Arc<Self>,
        now: Instant,
        ttl: Duration,
        build: F,
    ) -> Result<Arc<CatalogSearchSnapshot>, E>
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<CatalogSearchSnapshot, E>> + Send + 'static,
        E: std::fmt::Display + Send + 'static,
    {
        let mut slot = self.slot.lock().await;
        let existing = slot.entry.as_ref().map(|entry| {
            (
                Arc::clone(&entry.snapshot),
                now.saturating_duration_since(entry.built_at),
            )
        });
        match existing {
            // Younger than the TTL: the copy is served as it is.
            Some((snapshot, age)) if age < ttl => Ok(snapshot),
            // Older than the TTL, younger than MAX_STALE: the copy is
            // served at once and one background rebuild replaces it.
            Some((snapshot, age)) if age < MAX_STALE => {
                if !slot.rebuilding {
                    slot.rebuilding = true;
                    let generation = slot.generation;
                    let cache = Arc::clone(self);
                    tokio::spawn(async move {
                        let result = build().await;
                        let mut slot = cache.slot.lock().await;
                        slot.rebuilding = false;
                        match result {
                            Ok(snapshot) if slot.generation == generation => {
                                slot.entry = Some(Entry {
                                    snapshot: Arc::new(snapshot),
                                    built_at: now,
                                });
                            }
                            Ok(_) => {
                                // The copy was dropped or replaced while the
                                // rebuild ran: this one is older than what is
                                // in the slot now, so it goes.
                                tracing::debug!(
                                    "the background rebuild of the search copy landed after the copy was dropped; it is not kept"
                                );
                            }
                            Err(err) => {
                                // The old copy stays. The next search after
                                // the TTL starts another rebuild.
                                tracing::warn!(
                                    %err,
                                    "the background rebuild of the search copy failed; the old copy is kept"
                                );
                            }
                        }
                    });
                }
                Ok(snapshot)
            }
            // No copy, or one too old to serve: build it inline, single-
            // flight, as before. The lock is held across the build, so the
            // other searches wait for this one and read what it built.
            _ => {
                // Dropped before the build: if it fails, no old copy stays
                // behind.
                slot.entry = None;
                let snapshot = Arc::new(build().await?);
                slot.entry = Some(Entry {
                    snapshot: Arc::clone(&snapshot),
                    built_at: now,
                });
                slot.generation += 1;
                Ok(snapshot)
            }
        }
    }

    /// Drop the copy, so the next search rebuilds it inline (`DATA-38`
    /// D2).
    ///
    /// A background rebuild that is in flight does not put its copy back:
    /// the generation moves on, and the copy it built may have been read
    /// before the write that called this, so it must not outlive that
    /// write (`DATA-11` D2). An inline rebuild in flight holds the lock,
    /// so this waits for it and drops what it built.
    pub async fn invalidate(&self) {
        let mut slot = self.slot.lock().await;
        slot.entry = None;
        slot.generation += 1;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicU32, Ordering};

    use serde_json::json;

    use super::*;

    const TTL: Duration = Duration::from_secs(30);

    fn snapshot(tag: &str) -> CatalogSearchSnapshot {
        CatalogSearchSnapshot {
            assets: vec![json!({ "id": tag })],
            namespaces: json!([]),
            columns: HashMap::new(),
            usage: HashMap::new(),
            column_search_partial: false,
        }
    }

    /// A read that builds a copy tagged with the number of its build.
    async fn get(
        cache: &Arc<SearchSnapshotCache>,
        now: Instant,
        builds: &Arc<AtomicU32>,
    ) -> Arc<CatalogSearchSnapshot> {
        let builds = Arc::clone(builds);
        cache
            .get_or_build(now, TTL, move || {
                let builds = Arc::clone(&builds);
                async move {
                    let n = builds.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, String>(snapshot(&format!("build-{n}")))
                }
            })
            .await
            .unwrap()
    }

    /// A read whose rebuild always fails, counting the attempts.
    async fn failing_get(
        cache: &Arc<SearchSnapshotCache>,
        now: Instant,
        attempts: &Arc<AtomicU32>,
    ) -> Result<Arc<CatalogSearchSnapshot>, String> {
        let attempts = Arc::clone(attempts);
        cache
            .get_or_build(now, TTL, move || {
                let attempts = Arc::clone(&attempts);
                async move {
                    attempts.fetch_add(1, Ordering::SeqCst);
                    Err::<CatalogSearchSnapshot, String>("catalog unreachable".to_owned())
                }
            })
            .await
    }

    /// Waits until the background rebuild is done: the flag in the slot is
    /// clear again.
    ///
    /// The sleep is a poll interval for the task that runs the rebuild,
    /// not a wait for an age: the ages themselves come from the clock
    /// parameter and are never slept through.
    async fn wait_until_idle(cache: &Arc<SearchSnapshotCache>) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let rebuilding = cache.slot.lock().await.rebuilding;
                if !rebuilding {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the background rebuild must finish");
    }

    #[tokio::test]
    async fn the_first_read_builds_the_copy_inline() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let first = get(&cache, Instant::now(), &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(first.assets[0]["id"], "build-0");
    }

    #[tokio::test]
    async fn a_fresh_copy_is_served() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        let first = get(&cache, t0, &builds).await;
        let second = get(&cache, t0 + Duration::from_secs(29), &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn an_expired_copy_is_served_at_once_and_one_rebuild_starts() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        let first = get(&cache, t0, &builds).await;
        let second = get(&cache, t0 + TTL, &builds).await;
        assert!(
            Arc::ptr_eq(&first, &second),
            "an expired copy is served at once, while it is rebuilt behind it"
        );
        wait_until_idle(&cache).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2, "one rebuild runs");
    }

    #[tokio::test]
    async fn two_expired_reads_start_one_rebuild_not_two() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        get(&cache, t0, &builds).await;
        // Both reads find the same expired copy. The first starts the
        // rebuild; the second finds it running and starts no second one.
        let first = get(&cache, t0 + TTL, &builds).await;
        let second = get(&cache, t0 + TTL, &builds).await;
        assert!(Arc::ptr_eq(&first, &second));
        wait_until_idle(&cache).await;
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "the two reads share one rebuild"
        );
    }

    #[tokio::test]
    async fn a_successful_background_rebuild_swaps_the_copy_in() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        let first = get(&cache, t0, &builds).await;
        let second = get(&cache, t0 + TTL, &builds).await;
        assert!(Arc::ptr_eq(&first, &second));
        wait_until_idle(&cache).await;
        // The rebuild landed: the next read serves the new copy, and the
        // new copy is young enough that it starts no rebuild of its own.
        let third = get(&cache, t0 + TTL + Duration::from_secs(1), &builds).await;
        assert_eq!(third.assets[0]["id"], "build-1");
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "a fresh copy starts no rebuild"
        );
    }

    #[tokio::test]
    async fn a_failed_background_rebuild_keeps_the_old_copy_and_the_next_read_retries() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let attempts = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        let first = get(&cache, t0, &builds).await;
        // The copy is expired: a background rebuild starts, and it fails.
        let second = failing_get(&cache, t0 + TTL, &attempts).await.unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "the old copy is served while the rebuild runs"
        );
        wait_until_idle(&cache).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
        // The old copy is kept, so it is still expired: the next read tries
        // again.
        let third = failing_get(&cache, t0 + TTL + Duration::from_secs(1), &attempts)
            .await
            .unwrap();
        assert!(Arc::ptr_eq(&first, &third), "the old copy is kept");
        wait_until_idle(&cache).await;
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            2,
            "the next read after the TTL starts another rebuild"
        );
    }

    #[tokio::test]
    async fn a_copy_past_max_stale_is_rebuilt_inline() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        get(&cache, t0, &builds).await;
        // The copy is older than MAX_STALE: it is not served at all, and
        // the read builds a new one and waits for it.
        let later = get(&cache, t0 + MAX_STALE, &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert_eq!(later.assets[0]["id"], "build-1");
    }

    #[tokio::test]
    async fn after_invalidate_the_next_read_rebuilds_inline() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        get(&cache, t0, &builds).await;
        cache.invalidate().await;
        let after = get(&cache, t0, &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert_eq!(after.assets[0]["id"], "build-1");
    }

    #[tokio::test]
    async fn a_failed_inline_rebuild_returns_the_error_and_caches_nothing() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let attempts = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        // There is no copy: the first read builds it inline, and it fails.
        let failed = failing_get(&cache, t0, &attempts).await;
        assert_eq!(failed.unwrap_err(), "catalog unreachable");
        // Nothing was cached, so the next read builds again.
        let failed_again = failing_get(&cache, t0, &attempts).await;
        assert_eq!(failed_again.unwrap_err(), "catalog unreachable");
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn concurrent_searches_share_one_rebuild() {
        let cache = Arc::new(SearchSnapshotCache::new());
        let builds = Arc::new(AtomicU32::new(0));
        let t0 = Instant::now();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (cache, builds) = (Arc::clone(&cache), Arc::clone(&builds));
            tasks.push(tokio::spawn(async move {
                let builds = Arc::clone(&builds);
                cache
                    .get_or_build(t0, TTL, move || {
                        let builds = Arc::clone(&builds);
                        async move {
                            // A yield inside the build: without it the eight
                            // reads would not overlap.
                            tokio::task::yield_now().await;
                            let n = builds.fetch_add(1, Ordering::SeqCst);
                            Ok::<_, String>(snapshot(&format!("build-{n}")))
                        }
                    })
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "the eight reads share one rebuild"
        );
    }
}
