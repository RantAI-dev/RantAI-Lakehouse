//! Runs a dashboard's tiles side by side, inside the request (`BI-4` part A,
//! plan decisions D1 and D2; feature page decision 3).
//!
//! Before this module a dashboard request awaited its tiles one after another,
//! so a board took the sum of its tiles' queries and a board of slow tiles
//! could hit the 60-second request limit although each tile alone was quick.
//!
//! Two bounds apply, and both are needed:
//!
//! - **Per request: [`TILES_PER_REQUEST`] (6).** One board never opens more
//!   than six engine queries at once, however many tiles it has.
//! - **Per process: [`TILES_PER_PROCESS`] (24).** A semaphore on `AppState`
//!   (`tile_slots`, the `pdf_slots` pattern) that every request shares, so
//!   many people opening boards at once cannot flood `ClickHouse`. A tile
//!   holds its permit only while its own pipeline runs; a waiting tile holds
//!   nothing, so a request can never deadlock on its own tiles.
//!
//! The tiles are driven on the request's own task (no `tokio::spawn`): they
//! borrow the request's maps and the work is I/O. Results come back in the
//! order the tiles were given (`buffered`, not `buffer_unordered`), so the
//! `results` object of the payload lists its tiles in the same order as the
//! sequential code did and the JSON is identical. A tile reports its own
//! failure in its value; nothing here can fail one tile because of another.

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::FuturesOrdered;
use tokio::sync::Semaphore;

/// Tiles of one request computed at once (feature page decision 3).
pub const TILES_PER_REQUEST: usize = 6;

/// Tiles computed at once by one API process, across every request, unless
/// `TILE_CONCURRENCY_MAX` says otherwise.
pub const TILES_PER_PROCESS: usize = 24;

/// Smallest and largest value `TILE_CONCURRENCY_MAX` may take. Below the
/// per-request bound a single board could not use its six; above 128 the
/// engine's own connection limits, not this process, decide.
const PROCESS_LIMIT_RANGE: (usize, usize) = (4, 128);

/// The process bound from `TILE_CONCURRENCY_MAX`: whole number, clamped to
/// 4..=128 so a typo can neither stall every board nor remove the bound.
/// Unset, empty or unparseable gives [`TILES_PER_PROCESS`].
#[must_use]
pub fn process_limit_from(raw: Option<&str>) -> usize {
    raw.map(str::trim)
        .and_then(|s| s.parse::<usize>().ok())
        .map_or(TILES_PER_PROCESS, |n| {
            n.clamp(PROCESS_LIMIT_RANGE.0, PROCESS_LIMIT_RANGE.1)
        })
}

/// The semaphore `AppState::tile_slots` holds, sized from the environment.
/// Read once, when the state is built (like `AI_CHAT_TIMEOUT_SECS`).
#[must_use]
pub fn slots_from_env() -> Semaphore {
    Semaphore::new(process_limit_from(
        std::env::var("TILE_CONCURRENCY_MAX").ok().as_deref(),
    ))
}

/// Runs `tiles` with at most [`TILES_PER_REQUEST`] in flight, each one only
/// while it holds a permit of `slots`, and returns their outputs in the order
/// the tiles were given.
pub async fn run_tiles<'a, T>(slots: &'a Semaphore, tiles: Vec<BoxFuture<'a, T>>) -> Vec<T> {
    // `FuturesOrdered` with a refill by hand is `stream::iter(..).buffered(6)`
    // spelled out: the combinator's closure made the request handlers' futures
    // fail rustc's `Send` check ("not general enough" for `&AppState`).
    let mut waiting = tiles.into_iter();
    let mut running = FuturesOrdered::new();
    for tile in waiting.by_ref().take(TILES_PER_REQUEST) {
        running.push_back(run_one(slots, tile));
    }
    let mut out = Vec::new();
    while let Some(done) = running.next().await {
        out.push(done);
        if let Some(tile) = waiting.next() {
            running.push_back(run_one(slots, tile));
        }
    }
    out
}

/// [`run_tiles`] with a deadline (`BI-5` plan D4): a tile that has not finished
/// when `deadline` passes is dropped, which also ends its engine query (the
/// request carries `cancel_http_readonly_queries_on_client_close`), and its
/// place in the output is `unreached(index)`, so the caller answers the tiles
/// that did finish and says "not reached in time" for the rest instead of a
/// bare 408 (principle 2). A tile still waiting for a permit when the deadline
/// passes is not started.
pub async fn run_tiles_until<'a, T: 'a>(
    slots: &'a Semaphore,
    tiles: Vec<BoxFuture<'a, T>>,
    deadline: tokio::time::Instant,
    unreached: &'a (dyn Fn(usize) -> T + Send + Sync),
) -> Vec<T> {
    let bounded: Vec<BoxFuture<'a, T>> = tiles
        .into_iter()
        .enumerate()
        .map(|(i, tile)| {
            Box::pin(async move {
                match tokio::time::timeout_at(deadline, tile).await {
                    Ok(done) => done,
                    Err(_elapsed) => unreached(i),
                }
            }) as BoxFuture<'a, T>
        })
        .collect();
    run_tiles(slots, bounded).await
}

/// One tile under one permit. A named function, not a closure, so the
/// compiler keeps the tile's lifetime tied to `slots` (a closure here made the
/// request handlers' futures fail the `Send` check).
async fn run_one<'a, T>(slots: &'a Semaphore, tile: BoxFuture<'a, T>) -> T {
    // The semaphore is never closed (nothing calls `close`); if it ever were,
    // the tile still runs, unbounded, rather than being dropped: a missing
    // tile is worse than a slow board.
    let _permit = slots.acquire().await.ok();
    tile.await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;

    /// A tile that records how many tiles are in flight with it.
    fn tile<'a>(id: usize, now: &'a AtomicUsize, peak: &'a AtomicUsize) -> BoxFuture<'a, usize> {
        Box::pin(async move {
            let in_flight = now.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(in_flight, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(300)).await;
            now.fetch_sub(1, Ordering::SeqCst);
            id
        })
    }

    #[tokio::test(start_paused = true)]
    async fn twelve_tiles_take_two_rounds_never_more_than_six_at_once_and_keep_their_order() {
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let slots = Semaphore::new(TILES_PER_PROCESS);
        let started = tokio::time::Instant::now();
        let out = run_tiles(&slots, (0..12).map(|i| tile(i, &now, &peak)).collect()).await;
        assert_eq!(out, (0..12).collect::<Vec<_>>(), "order is the input order");
        assert_eq!(peak.load(Ordering::SeqCst), 6, "six at once, never more");
        // Twelve sequential 300 ms tiles take 3.6 s; two rounds of six take 600 ms.
        assert_eq!(started.elapsed(), Duration::from_millis(600));
    }

    #[tokio::test(start_paused = true)]
    async fn the_process_bound_holds_across_two_boards_at_once() {
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let slots = Semaphore::new(8);
        let board = |offset: usize| {
            let (now, peak) = (&now, &peak);
            let slots = &slots;
            async move {
                run_tiles(
                    slots,
                    (0..12).map(|i| tile(offset + i, now, peak)).collect(),
                )
                .await
            }
        };
        let (a, b) = tokio::join!(board(0), board(100));
        assert_eq!(a.len() + b.len(), 24);
        assert_eq!(
            peak.load(Ordering::SeqCst),
            8,
            "two boards of 12 share the 8 process slots, not 6 each"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiting_tile_holds_no_permit_so_a_small_bound_cannot_deadlock() {
        let (now, peak) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let slots = Arc::new(Semaphore::new(1));
        let out = run_tiles(&slots, (0..5).map(|i| tile(i, &now, &peak)).collect()).await;
        assert_eq!(out.len(), 5);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_process_limit_is_clamped_and_defaults_when_unset_or_bad() {
        assert_eq!(process_limit_from(None), 24);
        assert_eq!(process_limit_from(Some("")), 24);
        assert_eq!(process_limit_from(Some("many")), 24);
        assert_eq!(process_limit_from(Some(" 40 ")), 40);
        assert_eq!(process_limit_from(Some("1")), 4);
        assert_eq!(process_limit_from(Some("100000")), 128);
    }

    #[tokio::test(start_paused = true)]
    async fn tiles_past_the_deadline_are_replaced_by_the_unreached_value_in_order() {
        let slots = Semaphore::new(TILES_PER_PROCESS);
        let tiles: Vec<BoxFuture<'_, i64>> = (0..3_i64)
            .map(|i| {
                Box::pin(async move {
                    tokio::time::sleep(Duration::from_secs(if i == 1 { 100 } else { 1 })).await;
                    i
                }) as BoxFuture<'_, i64>
            })
            .collect();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let out = run_tiles_until(&slots, tiles, deadline, &|i| {
            -100 - i64::try_from(i).unwrap_or(0)
        })
        .await;
        assert_eq!(out, vec![0, -101, 2]);
    }
}
