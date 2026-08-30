//! Shared, hermetic Postgres test harness.
//!
//! Any integration test crate that depends on this crate (see
//! `lakehouse-store` and `lakehouse-auth`) gets a **`testcontainers`-managed
//! Postgres** started automatically, once per test binary, before any test
//! runs — no manual `docker compose up`, no shared external database.
//!
//! # How it works
//!
//! This crate uses [`ctor::ctor`] to run [`start_postgres_container`]
//! *before* `main()` — i.e. before the `libtest` harness spins up its
//! worker threads and before any `#[sqlx::test]` reads `DATABASE_URL`. That
//! sidesteps the usual "who initializes first" race between parallel test
//! threads: by the time any test body executes, `DATABASE_URL` already
//! points at a live, empty Postgres instance running in a container that
//! stays up for the lifetime of the test process.
//!
//! `#[sqlx::test(migrations = "../../migrations")]` then does the rest: for
//! *each* test it opens a fresh, migrated, isolated database against that
//! server and tears it down afterwards. We only need to provide the server.
//!
//! Just adding this crate as a dependency of a test binary is enough to
//! activate it; no call to any function here is required. [`database_url`]
//! is exposed for the rare test that needs to open its own connection
//! outside of `#[sqlx::test]`'s injected pool.
//!
//! # Requirements
//!
//! Docker (or a compatible container runtime) must be reachable from the
//! environment running `cargo test`. If it is not, the constructor panics
//! with a clear message instead of silently skipping tests.

use std::sync::OnceLock;

use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::postgres::Postgres;

static DATABASE_URL: OnceLock<String> = OnceLock::new();

/// Returns the `DATABASE_URL` of the per-process test Postgres container.
///
/// # Panics
///
/// Panics if called before the crate's `#[ctor]` hook has run, which should
/// never happen in practice: the hook runs before `main`.
#[must_use]
pub fn database_url() -> String {
    DATABASE_URL
        .get()
        .unwrap_or_else(|| panic!("lakehouse-test-support: DATABASE_URL not initialized yet"))
        .clone()
}

/// Starts (once per test process) a disposable Postgres container and
/// points `DATABASE_URL` at it.
///
/// Runs before `main` via `#[ctor::ctor]` below. Kept as a free function
/// (rather than inlined into the `#[ctor]` block) so it stays unit-testable
/// and readable.
fn start_postgres_container() {
    // A dedicated background runtime just for bringing the container up.
    // We intentionally leak it: it must keep driving the container's
    // internal async bookkeeping (e.g. log streaming) for the lifetime of
    // the test process, and processes that run `cargo test` always exit
    // shortly after the last test completes, so this is not a real leak in
    // practice.
    let runtime = Box::leak(Box::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap_or_else(|e| panic!("lakehouse-test-support: failed to start tokio runtime for the Postgres testcontainer: {e}")),
    ));

    let url = runtime.block_on(async {
        let container = Postgres::default()
            .with_tag("16-alpine")
            .start()
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "lakehouse-test-support: failed to start the Postgres testcontainer \
                     (is Docker running and reachable?): {e}"
                )
            });

        let host_port = container
            .get_host_port_ipv4(5432)
            .await
            .unwrap_or_else(|e| {
                panic!("lakehouse-test-support: failed to read the mapped Postgres port: {e}")
            });

        // Leak the container handle itself so it is never dropped (which
        // would stop and remove it) for the lifetime of the process.
        let container = Box::leak(Box::new(container));
        let _ = container;

        format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres")
    });

    DATABASE_URL
        .set(url.clone())
        .unwrap_or_else(|_| panic!("lakehouse-test-support: DATABASE_URL initialized twice"));

    // SAFETY: this runs from the `#[ctor]` constructor below, which fires
    // before `main` — before the test harness spawns any worker threads —
    // so there is no concurrent access to the process environment yet.
    unsafe {
        std::env::set_var("DATABASE_URL", &url);
    }
}

#[ctor::ctor]
fn init() {
    start_postgres_container();
}
