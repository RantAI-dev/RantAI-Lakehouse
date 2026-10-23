//! Shared, hermetic Postgres test harness.
//!
//! Any integration test crate that depends on this crate (see
//! `lakehouse-store` and `lakehouse-auth`) gets a **`testcontainers`-managed
//! Postgres** before any test runs — no manual `docker compose up`, no
//! shared external database. It is ONE labelled container reused by every
//! test process (started on first use, restarted if stopped), not a new one
//! per test binary: see `start_postgres_container`.
//!
//! # How it works
//!
//! This crate uses [`ctor::ctor`] to run [`start_postgres_container`]
//! *before* `main()` — i.e. before the `libtest` harness spins up its
//! worker threads and before any `#[sqlx::test]` reads `DATABASE_URL`. That
//! sidesteps the usual "who initializes first" race between parallel test
//! threads: by the time any test body executes, `DATABASE_URL` already
//! points at a live Postgres instance running in that shared container.
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

use testcontainers::runners::AsyncRunner;
use testcontainers::{ImageExt, ReuseDirective};
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
        // ONE container, reused by every test process: found by this label
        // and started again if it was stopped. The handle below is leaked
        // and nothing ever removed the container at exit, so the old
        // one-container-per-test-binary setup left every one of them
        // running — 245 on one shared dev VM (2026-09-22 → 26), about
        // 14 GiB of memory. Removing it at exit instead is not safe: tokio
        // cannot be driven after `main` has returned (it panics, and the
        // test process aborts). Sharing one server is fine for isolation,
        // since `#[sqlx::test]` gives every test its own database on it.
        //
        // OPS-10: server settings for a full `cargo test --workspace` run,
        // which drives several per-test databases and pools at once on this
        // one server:
        //
        // * `with_shm_size(1 GiB)` — Docker's default `/dev/shm` is 64 MiB.
        //   Postgres allocates dynamic shared memory (parallel query, and
        //   the per-query mmap segments behind it) there, so a burst of
        //   concurrent queries exhausts it and the server answers SQLSTATE
        //   53100 ("could not resize shared memory segment … No space left
        //   on device"), or resets the connection mid-handshake. The tests
        //   themselves are unchanged; the server now has room for the
        //   parallel work the same queries asked for.
        // * `max_parallel_workers_per_gather=0` — the single largest
        //   consumer of that dynamic shared memory per query. These are
        //   correctness tests over small tables; parallel scan buys them
        //   nothing and costs the shm segment.
        // * `max_connections=200` — one `#[sqlx::test]` pool is up to 10
        //   connections (sqlx's pool default) and a test binary runs as many
        //   of them as the machine has cores; `tests/common/mod.rs` adds a
        //   5-connection pool plus a 1-connection admin pool per test. On a
        //   6-core machine that worst case is 6 × (10 + 5 + 1) = 96, which
        //   the server default of 100 barely covers. The measured peak during
        //   the `lakehouse-api` lib binary (2,406 tests, the largest binary
        //   here, `RUST_TEST_THREADS=4`) was 15 client backends, so 200 is
        //   headroom rather than a live limit: pools open lazily and a test
        //   does not hold all of its connections at once.
        //
        // `with_cmd` replaces the image's command, so the `fsync=off` that
        // `testcontainers-modules`' `Postgres` sets is repeated here.
        //
        // The label value changed with these settings (`postgres-16` →
        // `postgres-16-shm1g`): a container started before this change is
        // reused by label and would keep the 64 MiB `/dev/shm`. A new name
        // means the old one is simply no longer found; remove it by hand
        // once (`docker rm -f $(docker ps -aq --filter
        // label=org.rantai.lakehouse-test-support)`).
        let container = Postgres::default()
            .with_tag("16-alpine")
            .with_cmd([
                "-c",
                "fsync=off",
                "-c",
                "max_parallel_workers_per_gather=0",
                "-c",
                "max_connections=200",
            ])
            .with_shm_size(1 << 30)
            .with_label("org.rantai.lakehouse-test-support", "postgres-16-shm1g")
            .with_reuse(ReuseDirective::Always)
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

        // Leak the handle so it is never dropped for the lifetime of the
        // process. (A reused container is not removed on drop either; the
        // leak keeps its log-streaming task alive.)
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

// `ctor` 1.0 requires an explicit acknowledgement that a pre-`main`
// constructor may do something unsafe (here: `set_var` in
// `start_postgres_container`, safe only because nothing else has started
// yet); see the SAFETY comment above.
#[ctor::ctor(unsafe)]
fn init() {
    start_postgres_container();
}
