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
//! shared server, under three prefixes:
//!
//! * `lakehouse_api_test_*` — one per `lakehouse-api` integration test
//!   (`rust/crates/lakehouse-api/tests/common/mod.rs:299`), which the
//!   test never drops. Since `OPS-23` the name carries the test
//!   process's own pid (`lakehouse_api_test_p<pid>_<uuid>`), so the
//!   sweep can tell a crashed run's database from a live test's;
//! * `lakehouse_api_main_test_*` — one per `lakehouse-api` binary crate's
//!   own unit test (`rust/crates/lakehouse-api/src/main.rs:1644`), which
//!   the test never drops either;
//! * `_sqlx_test_*` — sqlx 0.9's own per-test database, which sqlx drops
//!   only after a test that succeeded
//!   (`sqlx-core-0.9.0/src/testing/mod.rs:233-240` →
//!   `sqlx-postgres-0.9.0/src/testing/mod.rs:189-198`) — a failed or
//!   panicked test leaves its database behind.
//!
//! The server's volume reached 177.7 GB in three days and filled the dev
//! VM on 2026-10-26 (`OPS-13`).
//!
//! So this crate sweeps them, two rules side by side. Every 30 seconds
//! it drops the databases whose name carries one of the three test
//! prefixes and that were idle at two passes in a row — and, since
//! `OPS-23`, it also drops at the first pass any `lakehouse_api_test_`
//! database whose owner's pid is dead on this machine. A database is
//! idle at a pass when no backend was connected to it at that pass:
//! `pg_stat_activity` holds one row per backend, so a database with an
//! open connection is never idle. A database that was created a moment
//! ago and is not connected to yet is seen idle for the first time
//! and left alone.
//!
//! The dead-owner rule exists because the two-pass rule can never
//! catch a short run: the two sightings live in the memory of one
//! process, so a process that ends before its second pass drops
//! nothing, and a crashed or killed test's database then waited for a
//! later process that happened to live 30 seconds. The pid in the
//! name is what lets another process drop it at once: a dead owner
//! cannot be using the database, so the wait that protects a live
//! test's database does not apply to it. The drop uses
//! `WITH (FORCE)` because the connections of a killed process linger
//! on the server for a moment and the server refuses to drop a
//! database that is in use. A database whose owner is still alive is
//! never dropped by this rule, even when it is idle; the two-pass
//! rule applies to it as it does to every other name. Names without
//! a pid — old-style `lakehouse_api_test_` names,
//! `lakehouse_api_main_test_` and `_sqlx_test_` — keep the two-pass
//! rule unchanged.
//!
//! The rule is safe beside other batches: every test process on this
//! machine runs on the host in one pid namespace and shares one
//! container, so a live pid is a running test somewhere, possibly in
//! another batch's worktree, and its databases are left alone. Pid
//! reuse can only make a dead owner look alive, which keeps the
//! database and lets the two-pass rule catch it later. On CI each job
//! has a fresh container, so the rule changes nothing there. A pid
//! that cannot be checked — no `/proc` on this platform or an
//! unmounted one — counts as alive, never as dead.
//!
//! Nothing else on the server is ever dropped: not `postgres`, not
//! `template0`/`template1`, not a database whose name carries none of the
//! three prefixes. The sweep never fails a test — a failure is reported on
//! stderr, once per process, and the next pass tries again.
//!
//! The sweep is not a cleanup that can only help: it can drop a
//! database that a running test still needs. A database that had no
//! backend at two passes in a row is dropped, and the passes are 30
//! seconds apart, so a database nobody touches is taken away between
//! 30 and 60 seconds after its last query. A test that leaves its
//! database alone that long has it dropped underneath it, and the next
//! query fails with "database ... does not exist". Keep a query running
//! against the database, or keep the test short enough that the sweep
//! does not see the database as idle twice (`OPS-13` review fix
//! (SHOULD-FIX 2): the section did not state the rule the sweep imposes
//! on a test).
//!
//! A database can look idle while its test is still running. sqlx's test
//! pool closes an idle connection after one second (`idle_timeout = 1 s`,
//! `sqlx-postgres-0.9.0/src/testing/mod.rs:175-179`), so a database that
//! nobody touches has no backend on it at the next pass — idle in the
//! sense above — and the sweep cannot tell an abandoned database from a
//! paused test.
//!
//! A connection that is held open does keep the database alive:
//! `pg_stat_activity` shows the backend of an open connection, and a
//! database with a backend on it is never picked for dropping.
//!
//! Several test processes share the server at the same time. `cargo test`
//! runs one test binary at a time, with its tests on the threads of one
//! process. `cargo nextest` (CI since `REL-20`) gives every test a process
//! of its own and runs several at once, and a second checkout on the same
//! machine uses the same labelled container. Each test process that links
//! this crate runs its constructor, so each makes a first pass when it
//! starts and, if it lives that long, another pass every 30 seconds (this
//! crate's own test binary makes the first pass only, see
//! `spawn_sweep_loop`). The two-pass
//! state is in the memory of the process, so the passes of two processes
//! never add up: a database is dropped only by a process that saw it idle
//! twice itself, 30 seconds apart. A process that ends before its second
//! pass drops nothing — except the dead-owner rule above, which needs
//! no second pass.
//!
//! So a database that still has a backend on it is left alone by every
//! sweep, its own process's and another process's alike. A backend lasts
//! only as long as its connection is open, and the pool closes an idle
//! connection after one second, so a test that pauses for more than about
//! a second between two queries has no backend on its database in
//! between. A test must therefore not leave its database without a
//! backend for two passes in a row, between 30 and 60 seconds, or a
//! sweep, its own process's or another process's, may take it away.
//!
//! `OPS-22`: this crate's own tests are the one place where two passes
//! run with no time between them. Until that fix each of them swept the
//! whole server twice within milliseconds and dropped whatever had no
//! backend at that instant, a database another test had only just
//! created included (CI runs 43033910245, 43250741080, 43344075791,
//! 43349333080); the sentence that stood here, that the sweep of one
//! process can drop a database another process has just created, was
//! true of those tests only. A test's sweeper now sees only the
//! databases the test created (`Sweeper::only`), and the tests run side
//! by side.
//!
//! # Requirements
//!
//! Docker (or a compatible container runtime) must be reachable from the
//! environment running `cargo test`. If it is not, the constructor panics
//! with a clear message instead of silently skipping tests.

// `REL-14`: the one place that starts the `ClickHouse` testcontainer, with a
// bounded retry around a failed start (a broken image-pull stream used to
// fail whichever test started first). The Postgres harness below is a
// different arrangement — one reused, labelled container — and stays as is.
pub mod clickhouse;

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

/// How long the repeating sweep waits between two passes.
///
/// It is also the least time any process needs before its sweep can drop
/// a database: the two sightings of one sweeper are this far apart, and
/// each process has its own sweeper (`OPS-22`).
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

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
/// Three prefixes, each from a source of its own:
///
/// * `lakehouse_api_test_` — the per-test database of `lakehouse-api`'s
///   integration tests (`rust/crates/lakehouse-api/tests/common/mod.rs:299`),
///   which the test never drops.
/// * `lakehouse_api_main_test_` — the per-test database of `lakehouse-api`'s
///   binary crate's own unit tests
///   (`rust/crates/lakehouse-api/src/main.rs:1644`), which the test never
///   drops either. `OPS-13`: the sweep missed this third prefix, so 15
///   such databases (11 MB each) sat on the shared server after one
///   workspace test round, measured on 2026-10-28.
/// * `_sqlx_test_` — sqlx 0.9's own prefix
///   (`sqlx-core-0.9.0/src/testing/mod.rs:54`): the hash behind it is
///   base64url of a SHA-512 digest, so UPPERCASE letters and digits
///   occur, and the charset guard is `[A-Za-z0-9_]`, not `[a-z0-9_]`.
const SWEEPABLE_PREFIXES: [&str; 3] = [
    "lakehouse_api_test_",
    "lakehouse_api_main_test_",
    "_sqlx_test_",
];

/// The prefixes whose database names may carry the pid of the process that
/// created them (`OPS-23`).
///
/// Only `lakehouse_api_test_` is here: the harness of `lakehouse-api`'s
/// integration tests writes the pid of the test process into the name
/// (`rust/crates/lakehouse-api/tests/common/mod.rs`), so that the sweep of
/// another test process can drop the database of a crashed run at once.
///
/// The other two prefixes are not here, on purpose:
///
/// * `lakehouse_api_main_test_` — the binary crate's own unit tests write no
///   pid into their name;
/// * `_sqlx_test_` — sqlx writes that name itself: it is base64url of a
///   SHA-512 digest, so the part behind the prefix may hold almost
///   anything, and it must never be read as a pid.
const OWNER_PID_PREFIXES: [&str; 1] = ["lakehouse_api_test_"];

/// The pid of the process that created the database with this name, if the
/// name carries one.
///
/// The name carries a pid when it starts with one of
/// [`OWNER_PID_PREFIXES`] and the part behind the prefix reads
/// `p<digits>_<anything>`: the `p`, then the digits of the pid, then the
/// `_` that separates the pid from the rest of the name.
///
/// A part that is not of that shape is malformed and reads as "no pid":
/// the database then keeps the old two-pass rule. A malformed part is
/// never read as a number — not as `0`, not as any other pid — because a
/// name this crate did not write must never be dropped by the dead-owner
/// rule (`OPS-23`).
fn owner_pid(name: &str) -> Option<u32> {
    OWNER_PID_PREFIXES.iter().find_map(|prefix| {
        let rest = name.strip_prefix(prefix)?;
        parse_owner_pid(rest)
    })
}

/// The pid in the part of a name that follows an owner prefix.
fn parse_owner_pid(rest: &str) -> Option<u32> {
    let digits = rest.strip_prefix('p')?;
    let (pid, _rest) = digits.split_once('_')?;
    if !pid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Digits that do not fit a `u32` are malformed too: no process ever
    // had such a pid, so the name is not one the harness wrote.
    let pid = pid.parse::<u32>().ok()?;
    // Pid 0 is not a process: POSIX gives it the process-group meaning and
    // `/proc/0` does not exist, so a name that reads as pid 0 is a name
    // this crate did not write.
    if pid == 0 {
        return None;
    }
    Some(pid)
}

/// Whether the process with this pid is still running on this machine.
///
/// `OPS-23`: the answer decides whether the database of that process may be
/// dropped at once. It is the answer that must never be wrong in the
/// dangerous direction, so a pid that cannot be checked counts as alive:
///
/// * off Linux there is no `/proc` to look in, and this crate has no
///   dependency that could ask the kernel (`kill(pid, 0)` needs `libc` or
///   `nix`, and the workspace's `nix` has no `signal` feature);
/// * on Linux without a mounted `/proc` the pid cannot be checked either;
/// * a zombie still has its `/proc` entry, so it counts as alive. The
///   owner is not running any more, but the kernel has not reaped it yet,
///   and the old two-pass rule catches the database a moment later.
fn pid_is_alive(pid: u32) -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    let proc_dir = std::path::Path::new("/proc");
    if !proc_dir.is_dir() {
        return true;
    }
    proc_dir.join(pid.to_string()).exists()
}

/// Whether the owner of the database with this name is gone.
///
/// A name that carries no pid has no owner this crate can see, so it is
/// never "dead": it keeps the old two-pass rule.
fn owner_is_dead(name: &str) -> bool {
    owner_pid(name).is_some_and(|pid| !pid_is_alive(pid))
}

/// What a sweep pass does with one database.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    /// The pass drops the database.
    Drop,
    /// The pass leaves the database alone.
    Keep,
}

/// The decision for one database, from what the pass knows about it.
///
/// `idle_now` says the database had no backend at this pass, and
/// `idle_last_pass` says it had none at the previous pass. The two of them
/// are the rule the sweep has always had: a database nobody touches is
/// dropped between 30 and 60 seconds after its last query.
///
/// `OPS-23` adds one rule on top of it: the database of a process that is
/// gone is dropped at once, without waiting for the second pass. The wait
/// exists for a database whose owner may still be using it, and a dead
/// owner may not.
fn sweep_decision(name: &str, idle_now: bool, idle_last_pass: bool) -> Decision {
    if owner_is_dead(name) {
        return Decision::Drop;
    }
    if idle_now && idle_last_pass {
        return Decision::Drop;
    }
    Decision::Keep
}

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
///
/// `OPS-23`: a database whose owner process is gone is the one exception:
/// it is dropped at the first pass that sees it, without the second one.
struct Sweeper {
    /// The databases that had no connection on the previous pass.
    idle_seen_last_pass: BTreeSet<String>,
    /// `None`: every database on the server. That is the sweep of a test
    /// process (`start_sweep`), whose passes are `SWEEP_INTERVAL` apart.
    /// `Some`: only the databases with exactly these names (`OPS-22`, see
    /// `Sweeper::only`).
    only: Option<BTreeSet<String>>,
}

impl Sweeper {
    /// A sweeper that has not run a pass yet, and that looks at every
    /// database on the server.
    ///
    /// It is the server-wide sweeper, built by `start_sweep`. A test builds
    /// it for one pass at most, never two. `OPS-22`, two passes with no
    /// time between them would drop every test database that is idle at
    /// that instant, another test's included (see `Sweeper::only`).
    fn new() -> Self {
        Self {
            idle_seen_last_pass: BTreeSet::new(),
            only: None,
        }
    }

    /// A sweeper that looks only at the databases with these names.
    ///
    /// `OPS-22`: this crate's tests run two passes with no time between
    /// them. With the server-wide rule those two passes dropped every test
    /// database on the shared server that had no backend at that instant:
    /// the databases of the other tests of this binary (CI runs 43250741080,
    /// 43344075791, 43349333080) and the database of a running
    /// `#[sqlx::test]` of another binary (CI run 43033910245). `cargo
    /// nextest` gives every test a process of its own, so an in-process
    /// turn token cannot keep them apart. A test's sweeper is therefore
    /// given the names the test created and never sees another database.
    #[cfg(test)]
    fn only(names: &[&str]) -> Self {
        Self {
            idle_seen_last_pass: BTreeSet::new(),
            only: Some(names.iter().map(|name| (*name).to_owned()).collect()),
        }
    }

    /// Whether this sweeper may look at the database with this name.
    fn in_scope(&self, name: &str) -> bool {
        match &self.only {
            None => true,
            Some(names) => names.contains(name),
        }
    }

    /// Runs one pass: every database in this sweeper's scope (`in_scope`:
    /// all of them for the sweep of a test process) that carries one of the
    /// three test prefixes is dropped when its owner process is gone
    /// (`OPS-23`), or when nobody is connected to it and it was already
    /// idle on the previous pass.
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
        let mut dead_owner_candidates: BTreeSet<String> = BTreeSet::new();
        let mut two_pass_candidates: BTreeSet<String> = BTreeSet::new();

        for row in rows {
            let name: String = row.try_get("name")?;
            let busy: bool = row.try_get("busy")?;
            // OPS-22: a test's sweeper sees only the test's own databases.
            if !is_sweepable(&name) || !self.in_scope(&name) {
                continue;
            }
            // The two sightings the two-pass rule reads: the database had
            // no backend at this pass, and it had none at the previous one.
            let idle_at_this_pass = !busy;
            let idle_at_last_pass = self.idle_seen_last_pass.contains(&name);
            // The liveness of the owner is read twice, once here and once
            // inside `sweep_decision`. Two reads of `/proc` are cheap, and
            // they disagree only when the owner dies between them. If the
            // first read sees the owner alive and the second sees it dead,
            // `sweep_decision` returns `Drop` but the guard below does not
            // match, so the database takes the plain two-pass drop instead
            // of the forced one. A database whose killed owner still has a
            // backend on it fails that drop, and the next pass takes the
            // forced one. If the first read sees the owner dead and the
            // second sees it alive, `sweep_decision` drops the database
            // only when it was idle at two passes, and that forced drop is
            // still taken, because this read saw the owner dead. Otherwise
            // the pass keeps it and the next pass drops it. Neither mixed
            // read drops a live owner's database, because the forced drop
            // is taken only when this read saw the owner dead (PR review).
            let owner_dead = owner_is_dead(&name);
            match sweep_decision(&name, idle_at_this_pass, idle_at_last_pass) {
                // OPS-23: the owner is gone, so the database is dropped at
                // once. It may even still have a backend on it: the
                // connections of a process that was killed linger on the
                // server for a moment, which is why the drop of these
                // databases uses `WITH (FORCE)`.
                Decision::Drop if owner_dead => {
                    dead_owner_candidates.insert(name);
                }
                // The two-pass rule, unchanged: a database nobody touches
                // is dropped between 30 and 60 seconds after its last
                // query.
                Decision::Drop => {
                    idle_now.insert(name.clone());
                    two_pass_candidates.insert(name);
                }
                Decision::Keep => {
                    if idle_at_this_pass {
                        idle_now.insert(name);
                    }
                }
            }
        }

        // The next pass reads this set: the two-pass rule drops a database
        // only when it was idle at this pass AND at the previous one, so a
        // database created a moment ago and not yet connected to is seen
        // for the first time and left alone.
        //
        // A database of a dead owner is not in that set: it is dropped in
        // this same pass, and if the drop fails the dead-owner rule tries
        // again on the next pass, without any bookkeeping.
        self.idle_seen_last_pass = idle_now;

        // The databases of a dead owner go first: a pass that dies half
        // way through a long candidate list must not leave the databases of
        // a crashed run behind, and those are the urgent ones. Both
        // candidate sets are sorted, so the order is alphabetical today.
        for name in &dead_owner_candidates {
            // `WITH (FORCE)` closes the connections that are still open:
            // the owner is gone, but the server may not have noticed yet,
            // and it refuses to drop a database that is in use.
            //
            // `DROP DATABASE` takes an identifier, which cannot be bound as
            // a parameter. The name was checked by `is_sweepable` against
            // the three test prefixes and the `[A-Za-z0-9_]` charset, so it
            // cannot hold a quote, and `{:?}` writes it as a double-quoted
            // identifier with backslashes escaped — the same quoting
            // sqlx's own test harness uses (`sqlx-postgres-0.9.0/src/
            // testing/mod.rs:190`). That is why the statement may be built
            // with `format!` (`AGENTS.md`: "format! only for constant
            // identifiers").
            let statement = format!("DROP DATABASE {name:?} WITH (FORCE)");
            // A drop that fails is ignored: somebody connected to the
            // database in between, and the next pass tries again.
            let _ = sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&mut conn)
                .await;
        }
        // The two-pass candidates keep the plain drop of the sweep before
        // OPS-23, and the same quoting (see above).
        for name in &two_pass_candidates {
            let statement = format!("DROP DATABASE {name:?}");
            // A drop that fails is ignored: somebody connected to the
            // database in between, and the next pass tries again.
            let _ = sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&mut conn)
                .await;
        }
        Ok(dead_owner_candidates
            .into_iter()
            .chain(two_pass_candidates)
            .collect())
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
/// drive the passes explicitly, and a background sweeper would be one
/// more sweeper the tests have to reason about: it could drop a test's
/// database once the test is older than `SWEEP_INTERVAL`.
#[cfg(not(test))]
fn spawn_sweep_loop(url: String, mut sweeper: Sweeper) {
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

    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use sqlx::{Connection, PgConnection};

    use super::{
        Decision, SWEEP_INTERVAL, Sweeper, database_url, error_kind, is_sweepable, owner_pid,
        pid_is_alive, sweep_decision,
    };

    /// A pid that no process on this machine can have.
    ///
    /// Linux caps the pid at `pid_max` (4194304 by default), so
    /// `/proc/4294967295` cannot exist and the owner of a database named
    /// after that pid is gone.
    const DEAD_PID: u32 = u32::MAX;

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
    /// not drop is left to the sweep of another test process if it
    /// carries a test prefix, and stays for good if it does not
    /// (`untouched_db_…`). The tests drop their own databases at the end
    /// of the test; this guard is the net for a panic before that.
    ///
    /// No other test can pick these databases up in the meantime: a test's
    /// sweeper sees only the test's own databases (`OPS-22`).
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

    /// Asserts `holds`, unless the sweep of another process has had the time
    /// to change it.
    ///
    /// `OPS-22`: a test's own sweeper sees only the test's databases, but the
    /// databases carry the real prefixes, so the repeating sweep of another
    /// test process may drop one. That sweep needs two sightings
    /// `SWEEP_INTERVAL` apart, so nothing it does can be seen before the test
    /// is that old. `created` is taken before the test creates its first
    /// database. After `SWEEP_INTERVAL` the statement is no longer the
    /// sweeper's promise, and the line on stderr says it was not asserted.
    #[track_caller]
    fn assert_unless_another_sweep_had_time(created: Instant, holds: bool, what: &str) {
        if holds {
            return;
        }
        assert!(created.elapsed() >= SWEEP_INTERVAL, "{what}");
        eprintln!(
            "lakehouse-test-support: not asserted, the test is {} s old and the sweep of \
             another process may have dropped its database: {what}",
            created.elapsed().as_secs()
        );
    }

    #[test]
    fn only_a_test_prefix_and_a_plain_charset_may_be_dropped() {
        // sqlx's own names carry UPPERCASE letters and digits (the hash is
        // base64url), so the charset guard is `[A-Za-z0-9_]` and not
        // `[a-z0-9_]`.
        assert!(is_sweepable("_sqlx_test_aB3_xYz"));
        assert!(is_sweepable("lakehouse_api_test_0123456789abcdef"));
        assert!(is_sweepable("lakehouse_api_main_test_0123456789abcdef"));
        // No prefix, no drop: not `postgres`, not `template1`, not a name
        // outside the three prefixes.
        assert!(!is_sweepable("postgres"));
        assert!(!is_sweepable("template1"));
        assert!(!is_sweepable("untouched_db_1"));
        // A prefix alone is not enough: `LIKE` would match `Xsqlx_test_...`
        // too, and the name has to stay quotable as an identifier.
        assert!(!is_sweepable("Xsqlx_test_abc"));
        assert!(!is_sweepable("lakehouse_api_test_a-b"));
        assert!(!is_sweepable("lakehouse_api_test_a\"b"));
    }

    #[test]
    fn the_database_of_a_dead_owner_is_dropped_in_the_first_pass() {
        // OPS-23: the owner of the database is gone, so the two-pass wait
        // does not apply to it: the pass drops it at once.
        assert!(
            !pid_is_alive(DEAD_PID),
            "the liveness check must be able to tell that this pid is gone"
        );
        let name = format!("lakehouse_api_test_p{DEAD_PID}_0123456789abcdef");
        assert_eq!(
            sweep_decision(&name, true, false),
            Decision::Drop,
            "the database of a dead owner is dropped in the first pass"
        );
    }

    #[test]
    fn the_database_of_a_live_owner_is_kept() {
        // The owner of this database is this test process, which is alive:
        // the dead-owner rule does not fire, and the two-pass rule applies
        // to it as it does to every other name.
        let name = format!(
            "lakehouse_api_test_p{}_0123456789abcdef",
            std::process::id()
        );
        assert_eq!(
            sweep_decision(&name, true, false),
            Decision::Keep,
            "the database of a live owner is kept at the first pass"
        );
        assert_eq!(
            sweep_decision(&name, false, false),
            Decision::Keep,
            "the database of a live owner is kept while it is in use"
        );
        assert_eq!(
            sweep_decision(&name, true, true),
            Decision::Drop,
            "the two-pass rule still applies to the database of a live owner"
        );
    }

    #[test]
    fn an_old_style_name_keeps_the_two_pass_rule() {
        // A name without a pid keeps the rule of the sweep before OPS-23:
        // two idle passes, 30 seconds apart.
        let name = "lakehouse_api_test_0123456789abcdef";
        assert_eq!(
            sweep_decision(name, true, false),
            Decision::Keep,
            "the first sight of an old-style database leaves it alone"
        );
        assert_eq!(
            sweep_decision(name, false, false),
            Decision::Keep,
            "a database in use is kept"
        );
        assert_eq!(
            sweep_decision(name, false, true),
            Decision::Keep,
            "a database in use is kept"
        );
        assert_eq!(
            sweep_decision(name, true, true),
            Decision::Drop,
            "an old-style database is dropped after two idle passes"
        );
    }

    #[test]
    fn a_malformed_pid_part_is_treated_as_an_old_style_name() {
        // OPS-23: a name this crate did not write must never be read as a
        // pid, not even as the pid 0. Such a name keeps the two-pass rule.
        for name in [
            "lakehouse_api_test_p_0123456789abcdef",
            "lakehouse_api_test_px_0123456789abcdef",
            "lakehouse_api_test_p12",
            "lakehouse_api_test_p12a_0123456789abcdef",
            "lakehouse_api_test_p99999999999999_0123456789abcdef",
            // Pid 0 is not a process: it is not alive, so a name that reads
            // as pid 0 would be dropped as the database of a dead owner.
            "lakehouse_api_test_p0_0123456789abcdef",
        ] {
            assert_eq!(owner_pid(name), None, "{name} carries no pid");
            assert_eq!(
                sweep_decision(name, true, false),
                Decision::Keep,
                "{name} keeps the two-pass rule"
            );
            assert_eq!(
                sweep_decision(name, true, true),
                Decision::Drop,
                "{name} keeps the two-pass rule"
            );
        }
    }

    #[test]
    fn only_the_owner_prefix_names_carry_a_pid() {
        assert_eq!(
            owner_pid("lakehouse_api_test_p123_0123456789abcdef"),
            Some(123)
        );
        // The other two prefixes never carry an owner: sqlx writes that
        // name itself (base64url of a SHA-512 digest, so the part behind
        // the prefix may hold almost anything), and the binary crate's own
        // tests write no pid into their name.
        assert_eq!(owner_pid("_sqlx_test_p123_0123456789abcdef"), None);
        assert_eq!(
            owner_pid("lakehouse_api_main_test_p123_0123456789abcdef"),
            None
        );
        // A pid without the `_` that separates it from the rest of the name
        // is malformed: the name is not one the harness wrote.
        assert_eq!(owner_pid("lakehouse_api_test_p123"), None);
    }

    #[test]
    fn the_name_of_a_per_test_database_is_sweepable_and_short_enough() {
        // The name the harness of `lakehouse-api`'s integration tests
        // writes: `lakehouse_api_test_p` (20 bytes) + the pid (at most 7
        // digits) + `_` + the 32 hex characters of the UUID = 60 bytes,
        // inside Postgres's 63-byte identifier limit.
        let name = format!("lakehouse_api_test_p{}_{}", 4_194_304, "0".repeat(32));
        assert!(is_sweepable(&name), "{name} may be swept");
        assert!(name.len() <= 63, "{name} is too long for Postgres");
        assert_eq!(owner_pid(&name), Some(4_194_304));
    }

    #[tokio::test]
    async fn a_database_with_a_test_prefix_is_gone_after_two_passes() {
        let lakehouse_name = format!("lakehouse_api_test_{}", unique_suffix());
        let sqlx_name = format!("_sqlx_test_{}", unique_suffix());
        // The sweep below drops both databases; the guard is the net for
        // a panic before that.
        let _cleanup = Cleanup::new(&[lakehouse_name.as_str(), sqlx_name.as_str()]);
        let created = Instant::now();
        create_database(&lakehouse_name).await;
        create_database(&sqlx_name).await;

        let mut sweeper = Sweeper::only(&[lakehouse_name.as_str(), sqlx_name.as_str()]);
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert_unless_another_sweep_had_time(
            created,
            candidates.contains(&lakehouse_name),
            "a `lakehouse_api_test_` database with no connection is picked after two passes",
        );
        assert_unless_another_sweep_had_time(
            created,
            candidates.contains(&sqlx_name),
            "a `_sqlx_test_` database with no connection is picked after two passes",
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
    async fn a_database_with_the_bin_crate_prefix_is_gone_after_two_passes() {
        // OPS-13: the `lakehouse-api` binary's own unit tests create their
        // per-test database with a third prefix
        // (`rust/crates/lakehouse-api/src/main.rs:1644`) and never drop it,
        // so the sweep has to cover that prefix too.
        let name = format!("lakehouse_api_main_test_{}", unique_suffix());
        // The sweep below drops the database; the guard is the net for a
        // panic before that.
        let _cleanup = Cleanup::new(&[name.as_str()]);
        let created = Instant::now();
        create_database(&name).await;

        let mut sweeper = Sweeper::only(&[name.as_str()]);
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert_unless_another_sweep_had_time(
            created,
            candidates.contains(&name),
            "a `lakehouse_api_main_test_` database with no connection is picked after two passes",
        );
        assert!(
            !database_exists(&name).await,
            "a `lakehouse_api_main_test_` database with no connection is gone after two passes"
        );

        // The sweep has dropped the database already, so the drop is best
        // effort: it fails on a database that is gone.
        drop_database(&name).await;
    }

    #[tokio::test]
    async fn a_database_with_an_open_connection_survives_two_passes() {
        let name = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[name.as_str()]);
        create_database(&name).await;

        // The connection is held for the whole test: the database is in use,
        // so the sweep must leave it alone.
        let conn = PgConnection::connect(&database_url_for(&name))
            .await
            .expect("connect to the new test database");

        let mut sweeper = Sweeper::only(&[name.as_str()]);
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
        let name = format!("untouched_db_{}", unique_suffix());
        // The sweep never touches this name, so nothing else drops the
        // database: the guard is the only net for a panic before the drop
        // at the end of the test.
        let _cleanup = Cleanup::new(&[name.as_str()]);
        create_database(&name).await;

        // The scope holds the `untouched_db_…` name on purpose: only
        // `is_sweepable` keeps this database, which is what the test is
        // about (`OPS-22`).
        let mut sweeper = Sweeper::only(&[name.as_str()]);
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
            "only the three test prefixes may be picked for dropping"
        );
        assert!(
            database_exists(&name).await,
            "only the three test prefixes may be dropped"
        );

        drop_database(&name).await;
    }

    #[tokio::test]
    async fn the_cleanup_drops_the_database_of_a_test_that_panicked() {
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
        let name = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[name.as_str()]);
        let created = Instant::now();
        create_database(&name).await;

        let mut sweeper = Sweeper::only(&[name.as_str()]);
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");

        assert!(
            candidates.is_empty(),
            "the first sight of a database must leave it alone"
        );
        let name_exists = database_exists(&name).await;
        assert_unless_another_sweep_had_time(
            created,
            name_exists,
            "the first sight of a database must leave it alone",
        );

        drop_database(&name).await;
    }

    #[test]
    fn the_sweep_of_a_test_process_looks_at_every_database_and_a_tests_sweep_only_at_its_own() {
        // OPS-22: the scope is for this crate's tests only. The sweep of a
        // test process must still see every leftover database (`OPS-13`).
        let server_wide = Sweeper::new();
        assert!(server_wide.in_scope("_sqlx_test_anything"));
        assert!(server_wide.in_scope("lakehouse_api_test_anything"));

        let scoped = Sweeper::only(&["lakehouse_api_test_mine"]);
        assert!(scoped.in_scope("lakehouse_api_test_mine"));
        assert!(!scoped.in_scope("lakehouse_api_test_theirs"));
        assert!(!scoped.in_scope("_sqlx_test_anything"));
    }

    #[tokio::test]
    async fn one_pass_of_the_server_wide_sweep_sees_an_idle_test_database_and_drops_nothing() {
        // OPS-22 review (R1): the sweep of a test process (`Sweeper::new`) is
        // still driven through `pass` once. One pass can drop nothing: a new
        // sweeper has seen no database idle before.
        let name = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[name.as_str()]);
        let created = Instant::now();
        create_database(&name).await;

        let mut sweeper = Sweeper::new();
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the one server-wide pass");

        assert!(
            candidates.is_empty(),
            "the first pass of a sweeper picks nothing"
        );
        assert_unless_another_sweep_had_time(
            created,
            sweeper.idle_seen_last_pass.contains(&name),
            "the server-wide sweep sees an idle test database it was not given by name",
        );
        drop_database(&name).await;
    }

    #[tokio::test]
    async fn the_two_passes_of_another_test_between_this_tests_passes_take_nothing_from_it() {
        // OPS-22, CI runs 43250741080, 43344075791 and 43349333080: another
        // test of this binary ran its two passes between this test's two
        // passes and dropped this test's database, so this test's second
        // pass no longer found it. The interleaving is built here on purpose.
        let mine = format!("_sqlx_test_{}", unique_suffix());
        let theirs = format!("lakehouse_api_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[mine.as_str(), theirs.as_str()]);
        let created = Instant::now();
        create_database(&mine).await;
        create_database(&theirs).await;

        let mut my_sweeper = Sweeper::only(&[mine.as_str()]);
        let mut their_sweeper = Sweeper::only(&[theirs.as_str()]);
        my_sweeper
            .pass(&database_url())
            .await
            .expect("run this test's first pass");
        their_sweeper
            .pass(&database_url())
            .await
            .expect("run the other test's first pass");
        let their_candidates = their_sweeper
            .pass(&database_url())
            .await
            .expect("run the other test's second pass");
        let my_candidates = my_sweeper
            .pass(&database_url())
            .await
            .expect("run this test's second pass");

        assert_unless_another_sweep_had_time(
            created,
            my_candidates.contains(&mine),
            "a database is still picked by its own test after another test's two passes",
        );
        assert!(
            !their_candidates.contains(&mine),
            "another test's sweep never picks this test's database"
        );
        assert_unless_another_sweep_had_time(
            created,
            their_candidates.contains(&theirs),
            "the other test's sweep picks its own database",
        );
        assert!(
            !my_candidates.contains(&theirs),
            "this test's sweep never picks the other test's database"
        );
        assert!(
            !database_exists(&mine).await,
            "this test's database is gone"
        );
        assert!(
            !database_exists(&theirs).await,
            "the other test's database is gone"
        );

        drop_database(&mine).await;
        drop_database(&theirs).await;
    }

    #[tokio::test]
    async fn the_two_passes_of_a_test_leave_the_idle_database_of_another_test_alone() {
        // OPS-22, CI run 43033910245: the two passes of a test of this crate
        // dropped the database of a running `#[sqlx::test]` of another
        // binary (Postgres `3D000`). `theirs` stands for that database at a
        // moment it has no backend: sqlx creates it before it connects to
        // it, and its pool closes an idle connection after one second.
        let mine = format!("lakehouse_api_test_{}", unique_suffix());
        let theirs = format!("_sqlx_test_{}", unique_suffix());
        let _cleanup = Cleanup::new(&[mine.as_str(), theirs.as_str()]);
        let created = Instant::now();
        create_database(&mine).await;
        create_database(&theirs).await;

        let mut sweeper = Sweeper::only(&[mine.as_str()]);
        sweeper
            .pass(&database_url())
            .await
            .expect("run the first sweep pass");
        let candidates = sweeper
            .pass(&database_url())
            .await
            .expect("run the second sweep pass");

        assert!(
            !candidates.contains(&theirs),
            "a test's sweep never picks the idle database of another test"
        );
        let theirs_exists = database_exists(&theirs).await;
        assert_unless_another_sweep_had_time(
            created,
            theirs_exists,
            "the idle database of another test survives a test's two passes",
        );
        assert_unless_another_sweep_had_time(
            created,
            candidates.contains(&mine),
            "the test's own database is picked after two passes",
        );
        assert!(
            !database_exists(&mine).await,
            "the test's own database is gone"
        );

        drop_database(&theirs).await;
        drop_database(&mine).await;
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
