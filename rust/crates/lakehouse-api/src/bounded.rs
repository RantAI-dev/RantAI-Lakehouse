//! A bounded, best-effort fan-out over a set of keys.
//!
//! `GET /api/lakehouse/namespaces` and `GET /api/lakehouse/tables` each need
//! one Lakekeeper call per row (a namespace's table count, a table's
//! summary) and must not let a slow or hung catalog turn a list page into a
//! multi-minute request. [`run_with_budget`] runs every lookup concurrently,
//! bounded by `max_concurrent`, and stops waiting once `budget` elapses —
//! whatever finished by then is returned, and everything still in flight is
//! aborted rather than left running past the response.
//!
//! Built on a [`tokio::task::JoinSet`] plus a [`Semaphore`], not
//! `Arc<Mutex<...>>`/`blocking_lock`: an earlier draft collected results
//! into a shared `Arc<Mutex<HashMap>>` and, on timeout, tried
//! `Arc::try_unwrap` to reclaim it — which fails while any spawned task
//! still holds a clone, so the fallback `blocking_lock()` panicked inside
//! the async runtime the moment a lookup was still running at the deadline
//! (exactly the case this function exists to handle). Draining
//! `join_next()` under `timeout_at` and calling `abort_all()` on expiry
//! avoids that failure mode entirely: results are collected into a plain
//! `HashMap` owned by the caller, and nothing is shared across tasks except
//! the semaphore.

// This module is wired up by `GET /api/lakehouse/{namespaces,tables}`
// (`routes::lakehouse`, WS2 §4), landing in the next commit on this
// branch — until then, `run_with_budget` has no non-test caller.
#![allow(
    dead_code,
    reason = "consumed by routes::lakehouse in the next commit on this branch"
)]

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio::time::Instant;

/// Runs `lookup(key)` for every key in `keys`, at most `max_concurrent` at
/// once, and returns whatever finished within `budget`.
///
/// A key whose lookup does not finish in time, or whose lookup returns
/// `None`, is simply absent from the returned map — neither case is an
/// error. Lookups still running when the budget elapses are aborted; they
/// never keep running (and, for a Lakekeeper call, never keep talking to
/// the catalog) after this function returns.
pub(crate) async fn run_with_budget<K, V, F, Fut>(
    keys: Vec<K>,
    budget: Duration,
    max_concurrent: usize,
    lookup: F,
) -> HashMap<K, V>
where
    K: Eq + Hash + Clone + Send + 'static,
    V: Send + 'static,
    F: Fn(K) -> Fut,
    Fut: Future<Output = Option<V>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrent.max(1)));
    let mut set = JoinSet::new();
    for key in keys {
        let sem = semaphore.clone();
        let key_for_result = key.clone();
        // Calling `lookup(key)` here only builds the future; the lookup's
        // own body (and any network call it makes) does not run until the
        // spawned task below polls it, which happens only after the
        // permit is acquired.
        let fut = lookup(key);
        set.spawn(async move {
            let Ok(_permit) = sem.acquire_owned().await else {
                return (key_for_result, None);
            };
            (key_for_result, fut.await)
        });
    }

    let deadline = Instant::now() + budget;
    let mut results = HashMap::new();
    loop {
        match tokio::time::timeout_at(deadline, set.join_next()).await {
            Ok(Some(Ok((key, Some(value))))) => {
                results.insert(key, value);
            }
            // A lookup that returned `None`, or a task that panicked, is
            // simply absent from the map — not a reason to fail the batch.
            Ok(Some(Ok((_, None)) | Err(_))) => {}
            Ok(None) => break,
            Err(_elapsed) => {
                // The budget passed with tasks still in flight. Abort them
                // — they may still be waiting on a Lakekeeper response —
                // and return only what finished in time.
                set.abort_all();
                break;
            }
        }
    }
    results
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn slow_lookup_is_dropped_at_the_budget_and_its_task_is_aborted() {
        let finished = Arc::new(AtomicBool::new(false));
        let finished_task = finished.clone();

        let result = run_with_budget(vec!["slow"], Duration::from_millis(50), 4, move |_key| {
            let finished = finished_task.clone();
            async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                // Never reached: the budget (50ms) elapses long before
                // this 10s sleep does, and `run_with_budget` aborts the
                // task at the deadline rather than letting it finish.
                finished.store(true, Ordering::SeqCst);
                Some(1)
            }
        })
        .await;

        assert!(result.is_empty(), "the slow key must not appear in the map");
        assert!(
            !finished.load(Ordering::SeqCst),
            "the aborted task must never reach its completion marker"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn max_concurrent_is_respected() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let keys: Vec<u32> = (0..10).collect();

        let active_task = active.clone();
        let max_seen_task = max_seen.clone();
        let result = run_with_budget(keys, Duration::from_secs(5), 3, move |key| {
            let active = active_task.clone();
            let max_seen = max_seen_task.clone();
            async move {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                Some(key)
            }
        })
        .await;

        assert_eq!(result.len(), 10);
        assert!(
            max_seen.load(Ordering::SeqCst) <= 3,
            "at most 3 lookups should ever run concurrently, saw {}",
            max_seen.load(Ordering::SeqCst)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn all_keys_finish_within_budget() {
        let keys = vec![1_u32, 2, 3];

        let result = run_with_budget(keys, Duration::from_secs(1), 8, |key| async move {
            Some(key * 2)
        })
        .await;

        assert_eq!(result.len(), 3);
        assert_eq!(result.get(&1), Some(&2));
        assert_eq!(result.get(&2), Some(&4));
        assert_eq!(result.get(&3), Some(&6));
    }

    #[tokio::test(start_paused = true)]
    async fn a_lookup_returning_none_is_absent_from_the_map() {
        let result: HashMap<u32, u32> =
            run_with_budget(vec![1_u32], Duration::from_secs(1), 4, |_key| async move {
                None
            })
            .await;

        assert!(result.is_empty());
    }
}
