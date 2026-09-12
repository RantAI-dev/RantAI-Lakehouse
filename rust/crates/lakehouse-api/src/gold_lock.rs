//! Per-mart single-flight guard for `POST /api/gold/export/{mart}`
//! (`routes::gold::export`).
//!
//! Code-review defect: two concurrent `POST /api/gold/export/{mart}` calls
//! for the SAME mart both read `ClickHouse` and both append to Iceberg —
//! `GoldTable::append` has no dedup (`lakehouse-iceberg::gold`'s module doc
//! comment: export is append-only, `iceberg-rust` 0.10.x has no `UPDATE`/
//! `DELETE`), so a second concurrent run duplicates every row of the first
//! one's snapshot. [`MartLocks`] closes that: the SECOND concurrent caller
//! for a given mart fails to acquire the lock and gets `409 Conflict`
//! immediately, rather than either racing ahead (the bug) or queueing
//! behind the first call indefinitely (unbounded queueing — the scheduled
//! trigger `dagster/dispar_orchestrate/gold_export.py` uses has its own
//! timeout, and a caller blocked past it is no better off than one that
//! failed fast). A clear, immediate 409 is the chosen tradeoff; see
//! `routes::gold`'s module doc comment for where this is wired in.
//!
//! Different marts must still run concurrently — this is a per-mart lock,
//! not a single global one, so exporting `mart_a` and `mart_b` at the same
//! time is unaffected.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{Mutex, OwnedMutexGuard, TryLockError};

/// A registry of per-mart locks, created lazily the first time a given
/// mart name is exported. Cheap to clone (an `Arc` around the registry
/// itself, per [`crate::state::AppState`]'s "cheap to clone" contract) —
/// every clone shares the same underlying map.
#[derive(Clone, Default)]
pub struct MartLocks {
    inner: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl MartLocks {
    /// Try to acquire the lock for `mart` without waiting.
    ///
    /// On success, the returned guard must be held for the ENTIRE export
    /// (read from `ClickHouse` through the last Iceberg append) — the
    /// defect this exists to close is "both calls read AND both append",
    /// not just one half of it. Dropping the guard (including on an early
    /// error return, via RAII) releases the mart immediately for the next
    /// caller.
    ///
    /// The registry's own lock is held only long enough to look up (or
    /// insert) `mart`'s entry, never across the export itself — a slow
    /// export of `mart_a` never blocks a concurrent export of `mart_b`
    /// from even starting to check its own lock. Registry entries are
    /// never removed once created; this is bounded by the number of
    /// DISTINCT mart names ever exported on this process (a handful, in
    /// practice — Gold marts are a curated, human-named set, not a
    /// per-request identifier), not by the number of export calls, so it
    /// does not grow unbounded the way the defect this fixes would have.
    ///
    /// # Errors
    ///
    /// Returns [`TryLockError`] if another export of the SAME `mart` is
    /// already holding the lock. Callers should map this to `409 Conflict`
    /// — see `routes::gold::export`.
    pub async fn try_acquire(&self, mart: &str) -> Result<OwnedMutexGuard<()>, TryLockError> {
        let per_mart = {
            let mut registry = self.inner.lock().await;
            registry
                .entry(mart.to_owned())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        per_mart.try_lock_owned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The core guarantee this module exists to provide: a second
    /// concurrent export of the SAME mart must not proceed while the
    /// first is still running.
    #[tokio::test]
    async fn second_concurrent_export_of_same_mart_is_rejected() {
        let locks = MartLocks::default();
        let first = locks.try_acquire("sales_by_region").await.unwrap();

        let second = locks.try_acquire("sales_by_region").await;
        assert!(
            second.is_err(),
            "a second concurrent export of the same mart must fail to acquire the lock"
        );

        drop(first);
    }

    /// Different marts must still run concurrently — this is a per-mart
    /// lock, not one global export lock.
    #[tokio::test]
    async fn different_marts_export_concurrently() {
        let locks = MartLocks::default();
        let a = locks.try_acquire("mart_a").await;
        let b = locks.try_acquire("mart_b").await;
        assert!(a.is_ok(), "exporting mart_a must not be blocked by mart_b");
        assert!(b.is_ok(), "exporting mart_b must not be blocked by mart_a");
    }

    /// Releasing the guard (a completed or failed export) must free the
    /// mart for the next caller — single-flight, not "exported exactly
    /// once ever".
    #[tokio::test]
    async fn lock_is_released_after_guard_drop() {
        let locks = MartLocks::default();
        {
            let _guard = locks.try_acquire("sales_by_region").await.unwrap();
        }
        let reacquired = locks.try_acquire("sales_by_region").await;
        assert!(
            reacquired.is_ok(),
            "the mart must be exportable again once the previous guard is dropped"
        );
    }

    /// Two tasks racing `try_acquire` for the same mart at the same time:
    /// exactly one must win, proving this is safe under real concurrent
    /// scheduling, not just sequential calls on one task.
    #[tokio::test]
    async fn concurrent_racing_tasks_only_one_wins() {
        let locks = MartLocks::default();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));

        let mut tasks = Vec::new();
        for _ in 0..2 {
            let locks = locks.clone();
            let barrier = barrier.clone();
            // Each task returns the acquisition RESULT itself (guard
            // included), not a bool derived from it — collapsing to a
            // bool immediately (`.is_ok()`) would drop the winning guard
            // right there, releasing the lock before the other racing
            // task even attempts its own `try_acquire`, which is exactly
            // the kind of premature release that would let both "win"
            // nondeterministically and defeat the point of this test.
            tasks.push(tokio::spawn(async move {
                barrier.wait().await;
                locks.try_acquire("sales_by_region").await
            }));
        }

        // Every guard/error is collected BEFORE any of them are inspected
        // or dropped, so neither task's outcome can influence the other's
        // — this is what actually proves mutual exclusion under real
        // concurrent scheduling, not just sequential calls on one task.
        let mut results = Vec::new();
        for task in tasks {
            results.push(task.await.unwrap());
        }
        let wins = results.iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "exactly one of the two racing exports must win");
    }
}
