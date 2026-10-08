//! How many uploads may be in progress at once (`SEC-17`): the part that
//! lives in this process, the files being RECEIVED.
//!
//! Each file being received is held in memory (up to 50 MB, see
//! `routes::uploads::MAX_UPLOAD_BYTES`), so the count is a memory bound.
//! A place is taken BEFORE the body is read and given back when the guard
//! is dropped, which the handler's return does on every path, an error or a
//! cancelled request included. The count of LOADS is not here: it is the
//! rows in `ingesting`, counted in Postgres by
//! `lakehouse_store::uploads::mark_ingesting_within_limits`, so it holds
//! across API processes. The count here does not: with several API
//! processes each has its own (feature page `upload-limits-and-safe-csv`,
//! Limits).
//!
//! Nothing waits. A request over a limit is refused at once and the caller
//! sends it again.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use lakehouse_core::ApiError;

/// The sentence of every refusal of this module and of the load limit. One
/// constant so the two refusals cannot word it differently.
pub const TOO_MANY_UPLOADS: &str = "Too many uploads are in progress. Try again in a moment.";

/// Seconds a refused caller is told to wait (`Retry-After`).
pub const RETRY_AFTER_SECS: u64 = 5;

/// The 429 both limits answer with.
#[must_use]
pub fn too_many_uploads() -> ApiError {
    ApiError::TooManyRequests {
        message: TOO_MANY_UPLOADS.to_owned(),
        retry_after_secs: RETRY_AFTER_SECS,
    }
}

#[derive(Default)]
struct Counts {
    total: u32,
    per_user: HashMap<String, u32>,
}

/// Counts the files being received, per user and in total. Cheap to clone:
/// every clone shares one count, as [`crate::state::AppState`] requires.
#[derive(Clone)]
pub struct ReceiveLimiter {
    per_user: u32,
    total: u32,
    counts: Arc<Mutex<Counts>>,
}

impl ReceiveLimiter {
    /// A limiter with room for `per_user` files of one user and `total`
    /// files together.
    #[must_use]
    pub fn new(per_user: u32, total: u32) -> Self {
        Self {
            per_user,
            total,
            counts: Arc::new(Mutex::new(Counts::default())),
        }
    }

    /// Take a place for `user`, or `None` when the user or the installation
    /// is at its limit. The place is given back when the guard is dropped.
    #[must_use]
    pub fn try_acquire(&self, user: &str) -> Option<ReceiveGuard> {
        let mut counts = lock(&self.counts);
        let mine = counts.per_user.get(user).copied().unwrap_or(0);
        if mine >= self.per_user || counts.total >= self.total {
            return None;
        }
        counts.total += 1;
        counts.per_user.insert(user.to_owned(), mine + 1);
        Some(ReceiveGuard {
            user: user.to_owned(),
            counts: Arc::clone(&self.counts),
        })
    }
}

/// A poisoned lock still holds a usable count: nothing in this module
/// panics while holding it, and refusing every upload for ever because one
/// thread did is the worse failure.
fn lock(counts: &Mutex<Counts>) -> MutexGuard<'_, Counts> {
    counts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A place taken from a [`ReceiveLimiter`]; dropping it gives the place
/// back.
#[must_use = "the place is given back when the guard is dropped"]
pub struct ReceiveGuard {
    user: String,
    counts: Arc<Mutex<Counts>>,
}

impl Drop for ReceiveGuard {
    fn drop(&mut self) {
        let mut counts = lock(&self.counts);
        counts.total = counts.total.saturating_sub(1);
        if let Some(mine) = counts.per_user.get_mut(&self.user) {
            *mine = mine.saturating_sub(1);
            if *mine == 0 {
                counts.per_user.remove(&self.user);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_fifth_place_for_one_user_is_refused_and_another_user_is_admitted() {
        let limiter = ReceiveLimiter::new(4, 16);
        let held: Vec<_> = (0..4)
            .map(|_| limiter.try_acquire("ada").expect("within the limit"))
            .collect();
        assert!(limiter.try_acquire("ada").is_none());
        assert!(limiter.try_acquire("bob").is_some());
        drop(held);
    }

    #[test]
    fn the_seventeenth_place_in_total_is_refused() {
        let limiter = ReceiveLimiter::new(4, 16);
        let held: Vec<_> = (0..16)
            .map(|n| {
                limiter
                    .try_acquire(&format!("user-{}", n / 4))
                    .expect("within the limit")
            })
            .collect();
        assert!(limiter.try_acquire("someone-else").is_none());
        drop(held);
    }

    #[test]
    fn a_dropped_guard_gives_its_place_back() {
        let limiter = ReceiveLimiter::new(1, 1);
        let first = limiter.try_acquire("ada").expect("within the limit");
        assert!(limiter.try_acquire("ada").is_none());
        drop(first);
        assert!(limiter.try_acquire("ada").is_some());
    }

    #[test]
    fn a_clone_shares_the_count() {
        let limiter = ReceiveLimiter::new(1, 4);
        let other = limiter.clone();
        let _held = limiter.try_acquire("ada").expect("within the limit");
        assert!(other.try_acquire("ada").is_none());
    }

    #[test]
    fn the_refusal_is_a_429_with_the_fixed_sentence_and_a_retry_after() {
        match too_many_uploads() {
            ApiError::TooManyRequests {
                message,
                retry_after_secs,
            } => {
                assert_eq!(
                    message,
                    "Too many uploads are in progress. Try again in a moment."
                );
                assert_eq!(retry_after_secs, 5);
            }
            other => panic!("expected TooManyRequests, got {other:?}"),
        }
    }
}
