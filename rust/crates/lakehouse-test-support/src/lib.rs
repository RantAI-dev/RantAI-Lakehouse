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
//! # The container's server settings
//!
//! It runs with 1 GiB of `/dev/shm`, `max_parallel_workers_per_gather=0` and
//! `max_connections=200` (`OPS-10`): a full workspace run drives thousands of
//! per-test databases, migrations and pools at once against this one server,
//! and Docker's default 64 MiB `/dev/shm` ran out under it — Postgres answered
//! SQLSTATE 53100 ("could not resize shared memory segment … No space left on
//! device") in whichever binaries ran many per-test databases at once, while
//! each passed alone. The label value carries the settings
//! (`postgres-16-shm1g`) because a container is reused by label and one
//! started before this change would keep the old `/dev/shm`. The separate
//! "Connection reset by peer" failures in the same runs were a race with the
//! server's own start, not memory: see `wait_until_ready`. The tests
//! themselves did not change; the reasoning for each setting is at
//! `start_postgres_container`.
//!
//! # Leftover databases
//!
//! Every test process that links this crate leaves databases behind on the
//! shared server. `lakehouse-api`'s integration tests create one
//! `lakehouse_api_test_*` database per test and never drop it
//! (`rust/crates/lakehouse-api/tests/common/mod.rs`), and sqlx 0.9 drops
//! its own per-test database only after a test that succeeded
//! (`sqlx-core-0.9.0/src/testing/mod.rs:233-240` →
//! `sqlx-postgres-0.9.0/src/testing/mod.rs:189-198`) — a failed or
//! panicked test leaves its database behind. The server's volume reached
//! 177.7 GB in three days and filled the dev VM on 2026-10-26 (`OPS-13`).
//!
//! So this crate sweeps them: every 30 seconds it drops the databases
//! whose name carries one of the two test prefixes, that nobody is
//! connected to, and that were already idle on the previous pass. A
//! database that was created a moment ago and is not connected to yet is
//! left alone on the first pass, and a database with an open connection is
//! left alone altogether.
//!
//! Nothing else on the server is ever dropped: not `postgres`, not
//! `template0`/`template1`, not a database whose name carries neither
//! prefix. The sweep never fails a test — a failure is reported on
//! stderr, once per process, and the next pass tries again.
//!
//! The sweep is not a cleanup that can only help: it can drop a database
//! that a running test still needs. A test must therefore not go 30
//! seconds without touching its database. The sweep runs every 30
//! seconds and drops a database that was idle on two passes in a row, so
//! a test that creates a database and then leaves it alone for longer
//! than that has its database dropped underneath it, and the next query
//! fails with "database ... does not exist". Keep a query running
//! against the database, or keep the test short enough that the sweep's
//! next pass does not see the database as idle (`OPS-13` review fix
//! (SHOULD-FIX 2): the section did not state the rule the sweep imposes
//! on a test).
//!
//! A database can look idle while its test is still running. sqlx's test
//! pool closes an idle connection after one second (`idle_timeout = 1 s`,
//! `sqlx-postgres-0.9.0/src/testing/mod.rs:175-179`), so a database that
//! nobody touches has no backend on it, and the sweep cannot tell an
//! abandoned database from a paused test.
//!
//! A connection that is held open does keep the database alive:
//! `pg_stat_activity` shows the backend of an open connection, and a
//! database with a backend on it is never picked for dropping.
//!
//! Two test processes can share the server at the same time: `cargo test`
//! runs one test binary at a time, but two cargo processes (a second
//! worktree, another session) run against the same labelled container.
//! Each of them sweeps on its own schedule and with its own two-pass
//! state, so the sweep of one process can drop a database that a test of
//! another process has just created and has not connected to yet.
//!
//! That is the rule above, seen from the other side: a test that
//! connects to its database, and keeps touching it, is safe against the
//! sweep of its own process and against the sweep of another process
//! alike. A test that cannot connect right away cannot hold its
//! database against either of them: it must not leave the database
//! unconnected for more than about 30 seconds, or a sweep may take it
//! away. The tests of this crate therefore take a turn, one test at a
//! time, so that they do not drop each other's databases.
//!
//! # Requirements
//!
//! Docker (or a compatible container runtime) must be reachable from the
//! environment running `cargo test`. If it is not, the constructor panics
//! with a clear message instead of silently skipping tests.

use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sqlx::{Connection, Row};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ImageExt, ReuseDirective};
use testcontainers_modules::postgres::Postgres;

static DATABASE_URL: OnceLock<String> = OnceLock::new();

/// How long the server gets to answer a query after the container is started,
/// the pause between attempts, and the cap on one attempt. A *new* container
/// gets the same 60 s from `testcontainers` (`DEFAULT_STARTUP_TIMEOUT`); a
/// reused one used to get nothing (see [`wait_until_ready`]).
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_RETRY: Duration = Duration::from_millis(200);
const READY_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Waits until the server answers `SELECT 1`.
///
/// `testcontainers` 0.27.3 starts a stopped container it found by label and
/// returns without running the image's wait strategy
/// (`runners/async_runner.rs`, the reusable-containers branch:
/// `client.start_container(...)` then `ContainerAsync::construct`), while
/// `Postgres`'s readiness is the "database system is ready to accept
/// connections" message (`testcontainers-modules` 0.15,
/// `src/postgres/mod.rs`). So every test process that found the shared
/// container stopped ran while nothing listened yet, and the kernel reset
/// each setup connection: on 2026-10-23 the first full `lakehouse-api` lib
/// run of a session (container stopped) had 224 tests fail with
/// "Connection reset by peer" and the second (container running) had 0
/// (`OPS-10`). A TCP connect alone is not enough — the postmaster listens
/// before recovery ends and answers "the database system is starting up" —
/// and a query is exactly what `#[sqlx::test]` needs.
///
/// # Panics
///
/// Panics when no attempt answers within `READY_TIMEOUT`, with the last
/// error. A container that never becomes usable must fail loudly once, not
/// let every test report a reset connection of its own.
async fn wait_until_ready(url: &str) {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        let last = match tokio::time::timeout(READY_ATTEMPT_TIMEOUT, ready_once(url)).await {
            Ok(Ok(())) => return,
            Ok(Err(err)) => err,
            Err(_) => format!("no answer within {} s", READY_ATTEMPT_TIMEOUT.as_secs()),
        };
        assert!(
            tokio::time::Instant::now() < deadline,
            "lakehouse-test-support: the Postgres testcontainer did not answer a query \
             within {}s (last error: {last})",
            READY_TIMEOUT.as_secs()
        );
        tokio::time::sleep(READY_RETRY).await;
    }
}

/// One readiness attempt: connect and run `SELECT 1`.
async fn ready_once(url: &str) -> Result<(), String> {
    let mut conn = sqlx::PgConnection::connect(url)
        .await
        .map_err(|err| err.to_string())?;
    sqlx::query("SELECT 1")
        .execute(&mut conn)
        .await
        .map_err(|err| err.to_string())?;
    Ok(())
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

        let url = format!("postgres://postgres:postgres@127.0.0.1:{host_port}/postgres");
        // `OPS-10`: `start()` above starts a stopped container and returns
        // without waiting for Postgres, so the port can answer nothing for
        // the first seconds of the process (see `wait_until_ready`).
        wait_until_ready(&url).await;
        // `OPS-13`: the first sweep pass runs here, before `DATABASE_URL` is
        // set, so the sweep never depends on the environment: it is given
        // the URL of the server this process just started.
        start_sweep(&url).await;
        url
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

/// The database names the sweep may drop.
///
/// `lakehouse_api_test_` is the prefix `lakehouse-api`'s integration tests
/// give their per-test databases, which they never drop. `_sqlx_test_` is
/// sqlx 0.9's own prefix (`sqlx-core-0.9.0/src/testing/mod.rs:54`): the
/// hash behind it is base64url of a SHA-512 digest, so UPPERCASE letters
/// and digits occur, and the charset guard is `[A-Za-z0-9_]`, not
/// `[a-z0-9_]`.
const SWEEPABLE_PREFIXES: [&str; 2] = ["lakehouse_api_test_", "_sqlx_test_"];

/// Whether the sweep may drop the database with this name.
///
/// The prefix check is a plain `starts_with` and never SQL `LIKE`: `_` is a
/// single-character wildcard in `LIKE`, so `'_sqlx_test_%'` would also
/// match `Xsqlx_test_...`. The charset check keeps the name quotable as
/// an identifier (see [`Sweeper::pass`]).
fn is_sweepable(name: &str) -> bool {
    SWEEPABLE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Drops the test databases that pile up on the shared server.
///
/// `OPS-13`: the server's volume reached 177.7 GB in three days and filled
/// the dev VM on 2026-10-26 (see the module docs).
///
/// The state is the set of names that had no connection on the previous
/// pass: a database is only dropped once it was idle twice in a row.
struct Sweeper {
    /// The databases that had no connection on the previous pass.
    idle_seen_last_pass: BTreeSet<String>,
}

impl Sweeper {
    /// A sweeper that has not run a pass yet.
    fn new() -> Self {
        Self {
            idle_seen_last_pass: BTreeSet::new(),
        }
    }

    /// Runs one pass: every database that carries one of the two test
    /// prefixes, that nobody is connected to, and that was already idle on
    /// the previous pass is dropped.
    ///
    /// The databases the pass picked are returned, and not only the ones
    /// that are gone: the server itself refuses to drop a database that has
    /// an open connection, so without the list "the sweep left it alone"
    /// and "the sweep tried and the server refused" look the same.
    ///
    /// # Errors
    ///
    /// The error when the server could not be reached or the listing query
    /// failed. A *drop* that fails is not an error: somebody connected to
    /// the database in between, and the next pass tries again.
    async fn pass(&mut self, url: &str) -> Result<Vec<String>, sqlx::Error> {
        let mut conn = sqlx::PgConnection::connect(url).await?;
        // `pg_stat_activity` holds one row per backend of the whole cluster,
        // and its `datname` is the database that backend is connected to: a
        // database with a backend on it is in use. The `_sqlx_test` schema
        // that sqlx keeps inside the `postgres` database is a schema, not a
        // database, so it does not show up in `pg_database` at all.
        //
        // OPS-13 review fix (SHOULD-FIX 1): the drop never waits or stalls.
        // The check tells the pass which databases are in use, so the pass
        // offers the server only the databases that are genuinely idle.
        // Postgres refuses a `DROP DATABASE` on a database that is in use,
        // and the check is there so the pass does not rely on that refusal
        // and does not have to handle the error it raises.
        let rows = sqlx::query(
            "SELECT d.datname AS name,
                    EXISTS (SELECT 1 FROM pg_stat_activity a WHERE a.datname = d.datname) AS busy
             FROM pg_database d",
        )
        .fetch_all(&mut conn)
        .await?;

        let mut idle_now = BTreeSet::new();
        for row in rows {
            let name: String = row.try_get("name")?;
            let busy: bool = row.try_get("busy")?;
            if is_sweepable(&name) && !busy {
                idle_now.insert(name);
            }
        }

        // A database is dropped only when it was idle on this pass AND on
        // the previous one: a database created a moment ago and not yet
        // connected to is seen for the first time and left alone.
        let candidates: Vec<String> = idle_now
            .intersection(&self.idle_seen_last_pass)
            .cloned()
            .collect();
        self.idle_seen_last_pass = idle_now;

        for name in &candidates {
            // `DROP DATABASE` takes an identifier, which cannot be bound as
            // a parameter. The name was checked by `is_sweepable` against
            // the two test prefixes and the `[A-Za-z0-9_]` charset, so it
            // cannot hold a quote, and `{:?}` writes it as a double-quoted
            // identifier with backslashes escaped — the same quoting
            // sqlx's own test harness uses (`sqlx-postgres-0.9.0/src/
            // testing/mod.rs:190`). That is why the statement may be built
            // with `format!` (`AGENTS.md`: "format! only for constant
            // identifiers").
            let statement = format!("DROP DATABASE {name:?}");
            // A drop that fails is ignored: somebody connected to the
            // database in between, and the next pass tries again.
            let _ = sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&mut conn)
                .await;
        }
        Ok(candidates)
    }
}

/// Runs the first sweep pass and starts the repeating one.
async fn start_sweep(url: &str) {
    let mut sweeper = Sweeper::new();
    if let Err(err) = sweeper.pass(url).await {
        report_sweep_error(&err);
    }

    // The repeating pass belongs to the test process, not to this crate's
    // own test binary (see `spawn_sweep_loop`).
    #[cfg(not(test))]
    spawn_sweep_loop(url.to_owned(), sweeper);
}

/// The repeating sweep, for the life of the test process.
///
/// It is compiled out of this crate's own test binary: there the tests
/// drive the passes explicitly, and a background sweeper would race them —
/// it could drop a database between a test's creation and its first pass.
#[cfg(not(test))]
fn spawn_sweep_loop(url: String, mut sweeper: Sweeper) {
    /// How often the sweep runs.
    const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SWEEP_INTERVAL).await;
            if let Err(err) = sweeper.pass(&url).await {
                report_sweep_error(&err);
            }
        }
    });
}

/// Writes the first sweep failure of the process to stderr.
///
/// The sweep never fails a test: the failure is reported once, with the
/// error's kind and never the connection string, which carries the
/// password.
fn report_sweep_error(err: &sqlx::Error) {
    static REPORTED: AtomicBool = AtomicBool::new(false);
    if REPORTED.swap(true, Ordering::Relaxed) {
        return;
    }
    eprintln!(
        "lakehouse-test-support: the leftover-database sweep failed ({}); the tests are not affected",
        error_kind(err),
    );
}

/// The error's kind, without any text that could carry the connection
/// string (the URL holds the password).
fn error_kind(err: &sqlx::Error) -> String {
    let kind = match err {
        sqlx::Error::Configuration(_) => "configuration",
        sqlx::Error::Database(_) => "database",
        sqlx::Error::Io(_) => "io",
        sqlx::Error::Tls(_) => "tls",
        sqlx::Error::Protocol(_) => "protocol",
        _ => "other",
    };
    match err
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
    {
        Some(code) => format!("{kind}, SQLSTATE {code}"),
        None => kind.to_owned(),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use sqlx::{Connection, PgConnection};

    use super::{Sweeper, database_url, error_kind, is_sweepable};

    /// The turn token: the tests share one server and one sweep rule.
    ///
    /// Every sweep drops every idle test database it has seen twice, and a
    /// database that another test has just created is idle — so two tests
    /// running at once drop each other's databases. One test at a time,
    /// from the creation of its database to its last assertion.
    ///
    /// `tokio`'s `sync` feature (an async mutex) is not enabled in this
    /// crate, and a `std` mutex cannot be held across an `await`, so the
    /// turn is a flag the tests take with a compare-and-swap.
    static TURN: AtomicBool = AtomicBool::new(false);

    /// Holds the turn and gives it back when it is dropped, also when the
    /// test panicked.
    struct TurnGuard;

    impl Drop for TurnGuard {
        fn drop(&mut self) {
            TURN.store(false, Ordering::Release);
        }
    }

    /// Waits for the turn and takes it.
    async fn take_turn() -> TurnGuard {
        loop {
            if TURN
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return TurnGuard;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A suffix that makes a test database name unique and keeps it inside
    /// the `[A-Za-z0-9_]` charset the sweep allows.
    fn unique_suffix() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the Unix epoch")
            .as_nanos();
        format!("{}_{nanos}_{counter}", std::process::id())
    }

    /// The shared server's URL, pointing at another database on it.
    fn database_url_for(name: &str) -> String {
        let base = database_url();
        let (server, _) = base
            .rsplit_once('/')
            .expect("the test DATABASE_URL ends with a database name");
        format!("{server}/{name}")
    }

    /// Creates a database with this name on the shared server.
    async fn create_database(name: &str) {
        let mut conn = PgConnection::connect(&database_url())
            .await
            .expect("connect to the shared test server");
        sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name:?}")))
            .execute(&mut conn)
            .await
            .expect("create the test database");
        conn.close().await.expect("close the admin connection");
    }

    /// Whether a database with this name exists on the shared server.
    async fn database_exists(name: &str) -> bool {
        let mut conn = PgConnection::connect(&database_url())
            .await
            .expect("connect to the shared test server");
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_database WHERE datname = $1")
            .bind(name)
            .fetch_one(&mut conn)
            .await
            .expect("count the test databases");
        conn.close().await.expect("close the admin connection");
        count > 0
    }

    /// Drops a database, best effort: the sweep may have dropped it already.
    async fn drop_database(name: &str) {
        let mut conn = PgConnection::connect(&database_url())
            .await
            .expect("connect to the shared test server");
        let _ = sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE {name:?}")))
            .execute(&mut conn)
            .await;
        conn.close().await.expect("close the admin connection");
    }

    /// Drops a database without an async context.
    ///
    /// A `Drop` implementation cannot await, and a runtime cannot be
    /// entered from inside the context of another runtime, so the drop
    /// runs on its own thread with its own runtime.
    fn drop_database_blocking(name: &str) {
        let name = name.to_owned();
        let worker = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async move {
                // The cleanup must not hang the test binary: a server that
                // does not answer must not keep the thread waiting.
                let _ = tokio::time::timeout(Duration::from_secs(10), async move {
                    let Ok(mut conn) = PgConnection::connect(&database_url()).await else {
                        return;
                    };
                    // `WITH (FORCE)` closes the connections that are still
                    // open: the server refuses to drop a database that is in
                    // use, and a panic ends the test's connection without
                    // saying goodbye to it.
                    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
                        "DROP DATABASE {name:?} WITH (FORCE)"
                    )))
                    .execute(&mut conn)
                    .await;
                    let _ = conn.close().await;
                })
                .await;
            });
        });
        let _ = worker.join();
    }

    /// Drops the databases a test created, also when the test panicked.
    ///
    /// The sweep is compiled out of this crate's own test binary (see
    /// `spawn_sweep_loop`), so a database that a test creates and does
    /// not drop stays on the shared server for good. The tests drop their
    /// own databases at the end of the test; this guard is the net for a
    /// panic before that.
    ///
    /// The guard is declared after the turn token, so the cleanup runs
    /// while the test still holds the turn: no other test of this binary
    /// can pick the database up in the meantime.
    struct Cleanup {
        names: Vec<String>,
    }

    impl Cleanup {
        /// A cleanup for the databases with these names.
        fn new(names: &[&str]) -> Self {
            Self {
                names: names.iter().map(|name| (*name).to_owned()).collect(),
            }
        }
    }

    impl Drop for Cleanup {
        fn drop(&mut self) {
            for name in std::mem::take(&mut self.names) {
                drop_database_blocking(&name);
            }
        }
    }

    #[test]
    fn only_a_test_prefix_and_a_plain_charset_may_be_dropped() {
        // sqlx's own names carry UPPERCASE letters and digits (the hash is
        // base64url), so the charset guard is `[A-Za-z0-9_]` and not
        // `[a-z0-9_]`.
        assert!(is_sweepable("_sqlx_test_aB3_xYz"));
        assert!(is_sweepable("lakehouse_api_test_0123456789abcdef"));
        // No prefix, no drop: not `postgres`, not `template1`, not a name
        // outside the two prefixes.
        assert!(!is_sweepable("postgres"));
        assert!(!is_sweepable("template1"));
        assert!(!is_sweepable("untouched_db_1"));
        // A prefix alone is not enough: `LIKE` would match `Xsqlx_test_...`
        // too, and the name has to stay quotable as an identifier.
        assert!(!is_sweepable("Xsqlx_test_abc"));
        assert!(!is_sweepable("lakehouse_api_test_a-b"));
        assert!(!is_sweepable("lakehouse_api_test_a\"b"));
    }

    #[tokio::test]
    async fn a_database_with_a_test_prefix_is_gone_after_two_passes() {
        let _turn = take_turn().await;
        let lakehouse_name = format!("lakehouse_api_test_{}", unique_suffix());
        let sqlx_name = format!("_sqlx_test_{}", unique_suffix());
        // The sweep below drops both databases; the guard is the net for
        // a panic before that.
        let _cleanup = Cleanup::new(&[lakehouse_name.as_str(), sqlx_name.as_str()]);
        create_database(&lakehouse_name).await;
        create_database(&sqlx_name).await;

        let mut sweeper = Sweeper::new();
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert!(
            candidates.contains(&lakehouse_name),
            "a `lakehouse_api_test_` database with no connection is picked after two passes"
        );
        assert!(
            candidates.contains(&sqlx_name),
            "a `_sqlx_test_` database with no connection is picked after two passes"
        );
        assert!(
            !database_exists(&lakehouse_name).await,
            "a `lakehouse_api_test_` database with no connection is gone after two passes"
        );
        assert!(
            !database_exists(&sqlx_name).await,
            "a `_sqlx_test_` database with no connection is gone after two passes"
        );

        // The sweep has dropped both databases already, so the drop is
        // best effort: it fails on a database that is gone.
        drop_database(&lakehouse_name).await;
        drop_database(&sqlx_name).await;
    }

    #[tokio::test]
    async fn a_database_with_an_open_connection_survives_two_passes() {
        let _turn = take_turn().await;
        let name = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[name.as_str()]);
        create_database(&name).await;

        // The connection is held for the whole test: the database is in use,
        // so the sweep must leave it alone.
        let conn = PgConnection::connect(&database_url_for(&name))
            .await
            .expect("connect to the new test database");

        let mut sweeper = Sweeper::new();
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert!(
            !candidates.contains(&name),
            "a database with an open connection is never picked for dropping"
        );
        assert!(
            database_exists(&name).await,
            "a database with an open connection survives the sweep"
        );

        conn.close().await.expect("close the held connection");
        drop_database(&name).await;
    }

    #[tokio::test]
    async fn a_database_without_a_test_prefix_survives_two_passes() {
        let _turn = take_turn().await;
        let name = format!("untouched_db_{}", unique_suffix());
        // The sweep never touches this name, so nothing else drops the
        // database: the guard is the only net for a panic before the drop
        // at the end of the test.
        let _cleanup = Cleanup::new(&[name.as_str()]);
        create_database(&name).await;

        let mut sweeper = Sweeper::new();
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert!(
            !candidates.contains(&name),
            "only the two test prefixes may be picked for dropping"
        );
        assert!(
            database_exists(&name).await,
            "only the two test prefixes may be dropped"
        );

        drop_database(&name).await;
    }

    #[tokio::test]
    async fn the_cleanup_drops_the_database_of_a_test_that_panicked() {
        let _turn = take_turn().await;
        let name = format!("untouched_db_{}", unique_suffix());
        create_database(&name).await;

        // The panic is caught, so the test fails on the assertion below
        // and not on the panic itself. The name is one the sweep never
        // touches, so nothing but the cleanup can drop the database.
        let outcome = std::panic::catch_unwind(|| {
            let _cleanup = Cleanup::new(&[name.as_str()]);
            panic!("the test died");
        });
        assert!(outcome.is_err());

        assert!(
            !database_exists(&name).await,
            "the cleanup drops the database of a test that panicked"
        );
    }

    #[tokio::test]
    async fn one_pass_does_not_drop_a_database_seen_for_the_first_time() {
        let _turn = take_turn().await;
        let name = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[name.as_str()]);
        create_database(&name).await;

        let mut sweeper = Sweeper::new();
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");

        assert!(
            candidates.is_empty(),
            "the first sight of a database must leave it alone"
        );
        assert!(
            database_exists(&name).await,
            "the first sight of a database must leave it alone"
        );

        drop_database(&name).await;
    }

    #[test]
    fn the_sweep_error_carries_neither_the_connection_string_nor_the_password() {
        // OPS-13 review fix (SHOULD-FIX 4): no test pinned the promise
        // that the reported error carries neither the connection string
        // nor the password.
        //
        // The sweep reports a failure with the error's kind and never with
        // its own text: the text of a connection failure carries the URL,
        // and the URL carries the password.
        const URL_WITH_PASSWORD: &str = "postgres://postgres:postgres@127.0.0.1:5432/postgres";
        let err = sqlx::Error::Configuration(Box::new(std::io::Error::other(URL_WITH_PASSWORD)));

        // The fixture really carries the secret, so the assertions below
        // cannot pass for the wrong reason.
        assert!(
            err.to_string().contains(URL_WITH_PASSWORD),
            "the error's own text carries the connection string"
        );

        let kind = error_kind(&err);
        assert!(
            !kind.contains(URL_WITH_PASSWORD),
            "the reported kind carries no connection string"
        );
        assert!(
            !kind.contains("postgres"),
            "the reported kind carries no password"
        );
        assert!(
            kind.contains("configuration"),
            "the reported kind names the kind of the error"
        );
    }
}
