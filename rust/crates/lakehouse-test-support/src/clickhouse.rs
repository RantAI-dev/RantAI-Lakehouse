//! The shared `ClickHouse` testcontainer — one definition of its image, its
//! settings, and a bounded start retry (`REL-14`).
//!
//! Eight test modules used to build the same `ClickHouse::default()` with the
//! same env trio and `26.8` tag, each calling `.start().await` directly. When
//! the image is not in Docker's local cache, `start()` pulls it; a truncated
//! registry stream (`PullImage` … "bytes remaining on stream") failed
//! whichever test happened to start first, and a re-run passed. This module is
//! the one place that starts the image: a short, bounded retry covers the
//! transient pull failure on a developer machine and in jobs that do not
//! pre-pull, while CI's pre-pull step means the usual start needs no pull at
//! all.
//!
//! The container handle is returned to the caller so its lifetime stays with
//! the caller: most tests keep it for the whole test binary in a `OnceCell`,
//! `tests/ops_workloads.rs` drops it at the end of one test.

use std::future::Future;
use std::time::Duration;

use testcontainers::ContainerAsync;
use testcontainers::ImageExt;
use testcontainers::core::error::TestcontainersError;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::{CLICKHOUSE_PORT, ClickHouse};

/// The tag the compose deployment uses. `ClickHouse`'s own default is much
/// older, so every caller used to set this explicitly.
const IMAGE_TAG: &str = "26.8";

/// `REL-14`: three attempts, with a short pause between them, are enough for
/// a broken image-pull stream without turning a real Docker outage into a
/// long hang. Three is the same bound CI's pre-pull loop uses.
const START_ATTEMPTS: u8 = 3;

/// The pause between two start attempts.
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// The running container's handle, kept alive by the caller.
pub type Container = ContainerAsync<ClickHouse>;

/// The HTTP URL of a running container and the handle that keeps it alive.
pub type Started = (String, Container);

/// Starts the shared `ClickHouse`, retrying a failed start up to
/// [`START_ATTEMPTS`] times.
///
/// # Panics
///
/// Panics when no attempt succeeds, with the last error. A container that
/// cannot start must fail the test that needs it rather than let that test
/// dial an address nothing is listening on.
pub async fn start() -> Started {
    retry(START_ATTEMPTS, RETRY_DELAY, start_once)
        .await
        .unwrap_or_else(|err| {
            panic!(
                "lakehouse-test-support: failed to start the ClickHouse testcontainer \
                 in {START_ATTEMPTS} attempts: {err}"
            )
        })
}

/// One start attempt: build the image with the compose service's env trio,
/// start it, and read its HTTP address.
///
/// # Errors
///
/// The error when the container could not start (including a failed image
/// pull) or its address could not be read.
async fn start_once() -> Result<Started, TestcontainersError> {
    // ALL THREE env vars are required: the 26.8 image's entrypoint disables
    // network access for `default` when CLICKHOUSE_USER is unset — even with
    // an empty password — and CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT keeps
    // passwordless `default` reachable. The compose file carries the same
    // trio as its local-dev default; see `tests/gold_publication.rs`.
    let container = ClickHouse::default()
        .with_env_var("CLICKHOUSE_USER", "default")
        .with_env_var("CLICKHOUSE_PASSWORD", "")
        .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
        .with_tag(IMAGE_TAG)
        .start()
        .await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(CLICKHOUSE_PORT).await?;
    Ok((format!("http://{host}:{port}"), container))
}

/// Runs `operation` up to `attempts` times, pausing `delay` after a failure.
///
/// Every error is retried: `REL-14` is about a truncated image-pull stream
/// inside `start()`, but a readiness or port error from the same attempt is
/// just as transient and just as safe to try again. Each attempt builds a
/// fresh image request, so a half-pulled layer cannot poison the retry.
///
/// # Errors
///
/// The last attempt's error, unchanged.
async fn retry<T, E, F, Fut>(attempts: u8, delay: Duration, mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let mut attempt = 1_u8;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) if attempt < attempts => {
                eprintln!(
                    "lakehouse-test-support: attempt {attempt}/{attempts} failed: {err}; retrying"
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::cell::Cell;
    use std::time::Duration;

    use super::retry;

    #[tokio::test]
    async fn a_failed_attempt_is_retried_until_one_succeeds() {
        let calls = Cell::new(0_u8);
        let result: Result<u8, &str> = retry(3, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            let call = calls.get();
            async move {
                if call < 3 {
                    Err("bytes remaining on stream")
                } else {
                    Ok(call)
                }
            }
        })
        .await;

        assert_eq!(result, Ok(3));
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn the_last_error_is_returned_when_every_attempt_fails() {
        let calls = Cell::new(0_u8);
        let result: Result<(), &str> = retry(3, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            async { Err("the image pull broke") }
        })
        .await;

        assert_eq!(result, Err("the image pull broke"));
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test]
    async fn a_successful_first_attempt_is_not_retried() {
        let calls = Cell::new(0_u8);
        let result: Result<u8, &str> = retry(3, Duration::ZERO, || {
            calls.set(calls.get() + 1);
            async { Ok(7) }
        })
        .await;

        assert_eq!(result, Ok(7));
        assert_eq!(calls.get(), 1);
    }
}
