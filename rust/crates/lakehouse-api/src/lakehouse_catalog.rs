//! The shared, lazily-connected Lakekeeper Iceberg REST client used by
//! `routes::lakehouse`.
//!
//! # Why one shared, lazily connected client
//!
//! `IcebergClient::connect` performs a real `GET /v1/config` round trip
//! (`RestCatalogBuilder::load`). Connecting on every request would spend
//! that round trip on every page load and, for `GET /api/lakehouse/tables`,
//! eat into the 2s budget [`crate::bounded::run_with_budget`] gives the
//! per-table loads. Instead, [`AppState::iceberg`] holds one client,
//! connected on first use and reused after that.
//!
//! `iceberg-catalog-rest` 0.10.1 does not refresh its bearer token on its
//! own (its `client.rs` has an open "TODO: Support automatic token
//! refreshing"), and maps no HTTP 401 to a distinct error kind — an expired
//! `lakehouse-api-reader` token (ADR 0011: minted once, 30 days, never
//! re-minted by anything in this build) surfaces as an ordinary
//! [`RestError::Catalog`]. Rather than teach this crate to inspect
//! `iceberg-catalog-rest`'s error internals for a 401, [`call`] treats
//! every [`RestError::Catalog`] on a cached client as possibly stale: it
//! drops the cache, reconnects once, and retries the call once.
//! [`RestError::NotFound`] never triggers a reconnect — a missing table
//! isn't a credential problem. A connect failure is never cached: the next
//! request tries again rather than the service remembering "not
//! configured" forever.

// This module is wired up by `routes::lakehouse`'s five handlers (WS2 §4),
// landing in the next commit on this branch — until then, `client`/`call`
// have no non-test caller.
#![allow(
    dead_code,
    reason = "consumed by routes::lakehouse in the next commit on this branch"
)]

use std::future::Future;
use std::sync::Arc;

use lakehouse_iceberg::rest::RestError;
use lakehouse_iceberg::{IcebergClient, IcebergClientConfig, IcebergError};

use crate::gold_export::iceberg_config;
use crate::lakekeeper_token::read_token_file;
use crate::state::AppState;

/// The purpose label and env var [`read_token_file`] names in its fixed
/// 503 text when the reader token is unavailable — matches the token file
/// `docker-compose.yml`'s `lakekeeper-authz-init` mounts for this
/// principal (ADR 0011, WS2 task A0).
const READER_PURPOSE: &str = "lakehouse-api-reader";
const READER_TOKEN_ENV_VAR: &str = "LAKEKEEPER_READ_TOKEN_FILE";

/// Connects a fresh [`IcebergClient`] using the configured Lakekeeper
/// catalog URI/warehouse and the `lakehouse-api-reader` bearer token.
///
/// A failed token read or a failed connect both come back as
/// [`RestError::Catalog`] — both are "the catalog is not currently
/// reachable with this deployment's configuration", and `classify_rest_error`
/// (`routes::lakehouse`) turns either into the same fixed 503 text.
async fn connect(state: &AppState) -> Result<IcebergClient, RestError> {
    let token = read_token_file(
        &state.config.lakekeeper_read_token_file,
        READER_PURPOSE,
        READER_TOKEN_ENV_VAR,
    )
    .await
    .map_err(|err| RestError::Catalog(IcebergError::Catalog(err.to_string())))?;
    let config: IcebergClientConfig = iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );
    IcebergClient::connect(&config)
        .await
        .map_err(RestError::Catalog)
}

/// Returns the cached client, connecting one if none is cached yet.
///
/// Never caches a failed connect — the next call (this request, or the
/// next one) tries again.
///
/// # Errors
/// Returns [`RestError::Catalog`] if the token cannot be read or the
/// connect itself fails.
pub(crate) async fn client(state: &AppState) -> Result<Arc<IcebergClient>, RestError> {
    get_or_connect(&state.iceberg, || connect(state)).await
}

/// Runs `f` against the shared cached client, reconnecting once and
/// retrying once if `f` fails with [`RestError::Catalog`] (see the module
/// doc comment). [`RestError::NotFound`] is returned immediately, never
/// retried.
///
/// # Errors
/// Returns whatever `f` (or a failed reconnect) returns.
pub(crate) async fn call<T, F, Fut>(state: &AppState, f: F) -> Result<T, RestError>
where
    F: Fn(Arc<IcebergClient>) -> Fut,
    Fut: Future<Output = Result<T, RestError>>,
{
    call_with_retry(
        &state.iceberg,
        || connect(state),
        f,
        |err| matches!(err, RestError::Catalog(_)),
    )
    .await
}

/// Returns the cached client in `cache`, or connects and caches one via
/// `connect`.
async fn get_or_connect<C, E, ConnectFut>(
    cache: &tokio::sync::RwLock<Option<Arc<C>>>,
    connect: impl Fn() -> ConnectFut,
) -> Result<Arc<C>, E>
where
    ConnectFut: Future<Output = Result<C, E>>,
{
    if let Some(client) = cache.read().await.clone() {
        return Ok(client);
    }
    let mut guard = cache.write().await;
    // Re-check: another request may have connected while this one waited
    // for the write lock.
    if let Some(client) = guard.clone() {
        return Ok(client);
    }
    let client = Arc::new(connect().await?);
    *guard = Some(client.clone());
    Ok(client)
}

/// The retry policy itself, generic over the client and error types so it
/// is unit-testable with fakes — no live Lakekeeper, no real
/// [`IcebergClient`] — see the tests below.
async fn call_with_retry<C, T, E, ConnectFut, CallFut>(
    cache: &tokio::sync::RwLock<Option<Arc<C>>>,
    connect: impl Fn() -> ConnectFut,
    call: impl Fn(Arc<C>) -> CallFut,
    is_retryable: impl Fn(&E) -> bool,
) -> Result<T, E>
where
    ConnectFut: Future<Output = Result<C, E>>,
    CallFut: Future<Output = Result<T, E>>,
{
    let client = get_or_connect(cache, &connect).await?;
    match call(client).await {
        Ok(value) => Ok(value),
        Err(err) if is_retryable(&err) => {
            // The cached client may hold an expired credential — drop it
            // so `get_or_connect` reconnects rather than handing back the
            // same client that just failed.
            *cache.write().await = None;
            let fresh = get_or_connect(cache, &connect).await?;
            call(fresh).await
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::sync::RwLock;

    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    enum FakeError {
        /// Stands in for `RestError::Catalog` — retryable.
        Retryable,
        /// Stands in for `RestError::NotFound` — never retried.
        Terminal,
    }

    fn is_retryable(err: &FakeError) -> bool {
        matches!(err, FakeError::Retryable)
    }

    #[tokio::test]
    async fn a_successful_call_is_not_retried() {
        let cache: RwLock<Option<Arc<u32>>> = RwLock::new(None);
        let connect_calls = AtomicUsize::new(0);
        let call_calls = AtomicUsize::new(0);

        let result: Result<u32, FakeError> = call_with_retry(
            &cache,
            || {
                connect_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(1_u32) }
            },
            |client| {
                call_calls.fetch_add(1, Ordering::SeqCst);
                async move { Ok(*client * 10) }
            },
            is_retryable,
        )
        .await;

        assert_eq!(result, Ok(10));
        assert_eq!(connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(call_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_catalog_error_reconnects_once_and_succeeds() {
        let cache: RwLock<Option<Arc<u32>>> = RwLock::new(None);
        let connect_calls = AtomicUsize::new(0);
        let call_attempt = AtomicUsize::new(0);

        let result: Result<u32, FakeError> = call_with_retry(
            &cache,
            || {
                let n =
                    u32::try_from(connect_calls.fetch_add(1, Ordering::SeqCst)).unwrap_or(u32::MAX);
                async move { Ok(n + 1) }
            },
            |client| {
                let attempt = call_attempt.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Err(FakeError::Retryable)
                    } else {
                        Ok(*client * 100)
                    }
                }
            },
            is_retryable,
        )
        .await;

        assert_eq!(result, Ok(200));
        assert_eq!(
            connect_calls.load(Ordering::SeqCst),
            2,
            "expected one initial connect and exactly one reconnect"
        );
    }

    #[tokio::test]
    async fn not_found_is_never_retried() {
        let cache: RwLock<Option<Arc<u32>>> = RwLock::new(None);
        let connect_calls = AtomicUsize::new(0);
        let call_calls = AtomicUsize::new(0);

        let result: Result<u32, FakeError> = call_with_retry(
            &cache,
            || {
                connect_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(1_u32) }
            },
            |_client| {
                call_calls.fetch_add(1, Ordering::SeqCst);
                async { Err(FakeError::Terminal) }
            },
            is_retryable,
        )
        .await;

        assert_eq!(result, Err(FakeError::Terminal));
        assert_eq!(connect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(call_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_second_failure_after_reconnect_is_returned_as_an_error() {
        let cache: RwLock<Option<Arc<u32>>> = RwLock::new(None);
        let connect_calls = AtomicUsize::new(0);

        let result: Result<u32, FakeError> = call_with_retry(
            &cache,
            || {
                connect_calls.fetch_add(1, Ordering::SeqCst);
                async { Ok(1_u32) }
            },
            |_client| async { Err(FakeError::Retryable) },
            is_retryable,
        )
        .await;

        assert_eq!(result, Err(FakeError::Retryable));
        assert_eq!(
            connect_calls.load(Ordering::SeqCst),
            2,
            "a second failure still only reconnects once"
        );
    }

    #[tokio::test]
    async fn a_failed_connect_is_never_cached() {
        let cache: RwLock<Option<Arc<u32>>> = RwLock::new(None);
        let attempt = AtomicUsize::new(0);

        let first: Result<u32, FakeError> = get_or_connect(&cache, || {
            let n = attempt.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(FakeError::Retryable)
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .map(|client| *client);
        assert_eq!(first, Err(FakeError::Retryable));
        assert!(
            cache.read().await.is_none(),
            "a failed connect must not populate the cache"
        );

        let second: Result<u32, FakeError> = get_or_connect(&cache, || {
            let n = attempt.fetch_add(1, Ordering::SeqCst);
            async move {
                if n == 0 {
                    Err(FakeError::Retryable)
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .map(|client| *client);
        assert_eq!(second, Ok(42));
    }
}
