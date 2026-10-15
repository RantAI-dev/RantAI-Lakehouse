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
//! A copy older than the age given to [`SearchSnapshotCache::get_or_build`]
//! is rebuilt, by one request at a time: the others wait on the same lock
//! and then read the copy that request built, instead of each rebuilding.
//! A failed rebuild caches nothing, so an old copy is never served past
//! its age. A successful annotation write calls
//! [`SearchSnapshotCache::invalidate`], so a console edit shows at once.
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

/// The cache. Always present on [`crate::state::AppState`]; it needs no
/// external dependency.
///
/// A `tokio` mutex, not a `std` one: the lock is held across the rebuild's
/// `.await`s on purpose, which is what makes the rebuild single-flight.
#[derive(Default)]
pub struct SearchSnapshotCache {
    slot: Mutex<Option<Entry>>,
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
    /// # Errors
    ///
    /// Returns `build`'s error unchanged; nothing is cached then, and the
    /// next call builds again.
    pub async fn get_or_build<F, Fut, E>(
        &self,
        now: Instant,
        ttl: Duration,
        build: F,
    ) -> Result<Arc<CatalogSearchSnapshot>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<CatalogSearchSnapshot, E>>,
    {
        let mut slot = self.slot.lock().await;
        if let Some(entry) = slot.as_ref()
            && now.saturating_duration_since(entry.built_at) < ttl
        {
            return Ok(Arc::clone(&entry.snapshot));
        }
        // Dropped before the build: if it fails, no old copy stays behind.
        *slot = None;
        let snapshot = Arc::new(build().await?);
        *slot = Some(Entry {
            snapshot: Arc::clone(&snapshot),
            built_at: now,
        });
        Ok(snapshot)
    }

    /// Drop the copy, so the next search rebuilds. Waits for a rebuild in
    /// flight, so a copy built from data read before the write that called
    /// this cannot outlive it.
    pub async fn invalidate(&self) {
        *self.slot.lock().await = None;
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

    async fn get(
        cache: &SearchSnapshotCache,
        now: Instant,
        builds: &AtomicU32,
    ) -> Arc<CatalogSearchSnapshot> {
        cache
            .get_or_build(now, TTL, || async {
                let n = builds.fetch_add(1, Ordering::SeqCst);
                Ok::<_, String>(snapshot(&format!("build-{n}")))
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_fresh_copy_is_reused() {
        let (cache, builds, t0) = (
            SearchSnapshotCache::new(),
            AtomicU32::new(0),
            Instant::now(),
        );
        let first = get(&cache, t0, &builds).await;
        let second = get(&cache, t0 + Duration::from_secs(29), &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn an_old_copy_is_rebuilt() {
        let (cache, builds, t0) = (
            SearchSnapshotCache::new(),
            AtomicU32::new(0),
            Instant::now(),
        );
        get(&cache, t0, &builds).await;
        let later = get(&cache, t0 + TTL, &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert_eq!(later.assets[0]["id"], "build-1");
    }

    #[tokio::test]
    async fn an_invalidated_copy_forces_a_rebuild() {
        let (cache, builds, t0) = (
            SearchSnapshotCache::new(),
            AtomicU32::new(0),
            Instant::now(),
        );
        get(&cache, t0, &builds).await;
        cache.invalidate().await;
        get(&cache, t0, &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_failed_rebuild_returns_the_error_and_caches_nothing() {
        let (cache, builds, t0) = (
            SearchSnapshotCache::new(),
            AtomicU32::new(0),
            Instant::now(),
        );
        get(&cache, t0, &builds).await;
        // The copy is old, the rebuild fails: the error comes back and the
        // old copy is not served in its place.
        let failed = cache
            .get_or_build(t0 + TTL, TTL, || async {
                Err::<CatalogSearchSnapshot, _>("catalog unreachable".to_owned())
            })
            .await;
        assert_eq!(failed.unwrap_err(), "catalog unreachable");
        // Even a call at the old copy's own time must build, not find it.
        let again = get(&cache, t0, &builds).await;
        assert_eq!(builds.load(Ordering::SeqCst), 2);
        assert_eq!(again.assets[0]["id"], "build-1");
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
                cache
                    .get_or_build(t0, TTL, || async {
                        tokio::task::yield_now().await;
                        let n = builds.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(snapshot(&format!("build-{n}")))
                    })
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);
    }
}
