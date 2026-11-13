//! Shared harness for the HTTP-level (`tower::ServiceExt::oneshot`)
//! integration tests in this directory.
//!
//! Every test gets its own freshly created, freshly migrated Postgres
//! database (on the single `testcontainers` Postgres server
//! `lakehouse-test-support` starts once per test binary — see that
//! crate), and a real [`lakehouse_api::routes::router`] built from a real
//! [`lakehouse_api::state::AppState`]. `ClickHouse`/`Dagster`/the LLM are
//! never real: [`spin_up`] points every one of those URLs at
//! `http://127.0.0.1:1` (port 0/1 refuses a TCP connection immediately on
//! Linux, no DNS lookup, no timeout to wait out), so any handler whose
//! authorization passes and which then tries to reach one of those
//! upstreams fails fast and deterministically with a 502/503/500 rather
//! than hanging or reaching a real host — see the module docs on
//! `tests/route_auth.rs` for why that is exactly the behavior the authz
//! matrix needs (it only ever asserts "not 401/403", never a 2xx).
//!
//! The one exception is [`spin_up_without_env_llm_url`], which leaves the
//! LLM's URL out of the environment: the tests that must prove the CONSOLE
//! owns the assistant's provider address need the environment to say
//! nothing about it, because a set `LLM_URL` makes the environment own
//! that address (`ADM-9 review R4`).

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::HashMap;

use lakehouse_api::config::Config;
use lakehouse_api::state::AppState;
use lakehouse_auth::Secret;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped by
// the linker before its ctor section is ever considered — see that
// crate's doc comment).
use lakehouse_test_support as _;

#[path = "../../src/tls_proxy.rs"]
pub mod tls_proxy;

/// A running [`AppState`]/[`axum::Router`] pair over an isolated,
/// migrated-and-seeded test database, plus the raw pool for direct setup
/// (minting sessions, inserting throwaway fixtures) that goes around the
/// HTTP surface entirely.
pub struct TestApp {
    pub router: axum::Router,
    pub pool: PgPool,
}

/// An unreachable-but-instantly-refusing HTTP origin — see the module doc
/// comment for why this, rather than a real mock server, is the right
/// default for every upstream this crate does not itself own.
const DEAD_UPSTREAM: &str = "http://127.0.0.1:1";

/// The harness's dead upstream is in the deployment's environment: the
/// default for every test that does not say otherwise.
const DEAD_LLM_URL: bool = true;

/// The deployment's environment says nothing about the assistant's
/// provider address, so the console owns it (`ADM-9 review R4`).
const NO_ENV_LLM_URL: bool = false;

/// `SRC-12` decision 4: the installation key every test app is given unless
/// a test overrides `CONNECTOR_SECRETS_KEY`: base64 of 32 bytes of `0x2a`.
/// Not a secret; it protects nothing outside a test's temporary directory.
/// SEC-36: encoded at run time, so the repository holds no key-shaped literal
/// for the `generic-api-key` rule to match.
pub fn test_connector_secrets_key() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode([0x2a_u8; 32])
}

/// The credential the API stored in `dir` under `file_name`, opened with
/// [`test_connector_secrets_key`]. Panics if the file is not a seal under
/// that key (which is what a test asserting "stored encrypted" wants).
pub fn stored_credential(dir: &std::path::Path, file_name: &str) -> String {
    let bytes = std::fs::read(dir.join(file_name)).unwrap();
    let ring = lakehouse_api::secret_crypto::Keyring::from_values(
        Some(&test_connector_secrets_key()),
        None,
    )
    .unwrap();
    String::from_utf8(ring.open(file_name, &bytes).unwrap().plaintext).unwrap()
}

/// Creates a brand-new, randomly named database on the shared test
/// Postgres container, migrates it (`0002_seed_identity.sql` and friends
/// run as part of that), and returns a [`TestApp`] wired to it.
///
/// Each call is fully isolated from every other: two tests calling this
/// concurrently never see each other's rows, matching the isolation
/// `#[sqlx::test]` gives `lakehouse-store`/`lakehouse-auth`'s tests (see
/// `lakehouse-test-support`), just done by hand here because
/// `AppState::new` — not a `PgPool` extractor — is what this crate's tests
/// need.
pub async fn spin_up() -> TestApp {
    spin_up_with_env(&HashMap::new()).await
}

/// Same as [`spin_up`], but `overrides` is merged into the env map AFTER
/// the usual dead-upstream defaults are set, so a caller can point one
/// upstream at something real (e.g. `LLM_URL` at a `wiremock::MockServer`,
/// for T3.2's headless-run tests that actually need the LLM to answer)
/// without losing the others' fail-fast behaviour, and can set config-only
/// keys (e.g. `AGENT_RUN_TOKEN`) that [`spin_up`] never sets at all.
pub async fn spin_up_with_env(overrides: &HashMap<String, String>) -> TestApp {
    let (state, pool) = build_state_and_pool(overrides, DEAD_LLM_URL).await;
    let router = lakehouse_api::routes::router(state);
    TestApp { router, pool }
}

/// Same as [`spin_up_with_env`], but the harness's dead `LLM_URL` is left
/// OUT of the deployment's environment, so the environment says nothing
/// about the assistant's provider address and the console owns it
/// (`ADM-9 review R4`).
///
/// The harness points `LLM_URL` at a dead upstream so a test that reaches
/// for the model fails fast instead of calling a real provider. A test
/// that must prove the CONSOLE owns the address needs the opposite: with
/// `LLM_URL` set, the environment owns the address, the console shows it
/// read-only, and a save that sends it is refused with a 409.
///
/// The address this deployment starts with is the code's own default, so a
/// test in it must not reach for the model before it has saved a provider
/// address of its own.
pub async fn spin_up_without_env_llm_url(overrides: &HashMap<String, String>) -> TestApp {
    let (state, pool) = build_state_and_pool(overrides, NO_ENV_LLM_URL).await;
    let router = lakehouse_api::routes::router(state);
    TestApp { router, pool }
}

/// The [`AppState`] and the migrated per-test database behind
/// [`spin_up_with_env`], without a router: for a test that calls a
/// background job (the scheduled-report runner, `RPT-1`) directly.
pub async fn spin_up_state(overrides: &HashMap<String, String>) -> (AppState, PgPool) {
    build_state_and_pool(overrides, DEAD_LLM_URL).await
}

/// Same as [`spin_up_with_env`], but the [`AppState`] the router was built
/// from is returned as well, and `prepare` may change it before the router
/// is built: a test that counts what the API did — the child runs of
/// `SEC-22` — needs the handle, and a test that points the runner at
/// another child needs to do it before the router takes the state.
pub async fn spin_up_with_state(
    overrides: &HashMap<String, String>,
    prepare: impl FnOnce(&mut AppState),
) -> (TestApp, AppState) {
    let (mut state, pool) = build_state_and_pool(overrides, DEAD_LLM_URL).await;
    prepare(&mut state);
    let router = lakehouse_api::routes::router(state.clone());
    (TestApp { router, pool }, state)
}

/// Same as [`spin_up_with_state`], but the harness's dead `LLM_URL` is left
/// out of the deployment's environment, so the environment says nothing
/// about the assistant's provider address and the console owns it
/// (`ADM-9 review R4`) — see [`spin_up_without_env_llm_url`].
pub async fn spin_up_with_state_without_env_llm_url(
    overrides: &HashMap<String, String>,
    prepare: impl FnOnce(&mut AppState),
) -> (TestApp, AppState) {
    let (mut state, pool) = build_state_and_pool(overrides, NO_ENV_LLM_URL).await;
    prepare(&mut state);
    let router = lakehouse_api::routes::router(state.clone());
    (TestApp { router, pool }, state)
}

/// Same as [`spin_up_with_env`], but replaces
/// [`AppState::connector_secret_resolver`] with `resolver` before the
/// router is built.
///
/// Exists for the ONE test that needs to prove a derived connector-credential
/// name (ADR 0002 Addendum 3) actually resolves and a `PUT
/// /api/connectors/{id}/secret` rotation lands
/// (`connector_secret_rotate.rs`'s `a_derived_secret_ref_resolves_and_the_rotation_lands`)
/// — without calling `std::env::set_var` (which would mutate process-wide
/// state shared with every other test in this binary, running
/// concurrently) and without provisioning a real `/run/secrets` file. The
/// resolver is swapped on the already-built [`AppState`], the same way
/// `routes::connectors`'s own unit tests
/// (`state_with_stub_secret_resolver`) do it — that field is `pub` for
/// exactly this reason.
pub async fn spin_up_with_connector_secret_resolver(
    overrides: &HashMap<String, String>,
    resolver: std::sync::Arc<dyn lakehouse_core::secret::DynSecretResolver>,
) -> TestApp {
    let (mut state, pool) = build_state_and_pool(overrides, DEAD_LLM_URL).await;
    state.connector_secret_resolver = resolver;
    let router = lakehouse_api::routes::router(state);
    TestApp { router, pool }
}

/// Same as [`spin_up_with_env`], but the pool the ROUTES see
/// ([`AppState::pg`], swapped after `AppState::new` — the same swap the
/// resolver variant above does) is a **one-connection pool over the same
/// fresh database that counts every connection checkout** in the returned
/// [`std::sync::atomic::AtomicUsize`], via
/// [`sqlx::pool::PoolOptions::before_acquire`]. The returned
/// [`TestApp::pool`] is the ORIGINAL pool, so the caller's own setup writes
/// (fixtures, sessions) go around the counter and only what a request does
/// is measured.
///
/// The delta across one request is the number of separate connection
/// checkouts it made, which is the `N+1` shape
/// `BLD-2 review R5 SHOULD-FIX 1` is about.
///
/// Two things it is NOT, and why: it does not count statements (sqlx's own
/// `sqlx::query` tracing events cannot be captured reliably here — the tests
/// of one binary run in parallel in one process and a callsite's `Interest`
/// is cached process-wide the first time it is hit, so a subscriber
/// registered later captures nothing), and it does not use
/// `pg_stat_database.xact_commit` (measured on this harness: a request's
/// statements are not in that counter yet when the next query asks for it,
/// so the delta reads 0).
///
/// Determinism: sqlx counts neither a new physical connection nor
/// [`PoolOptions::before_acquire`] on one, so the warm-up `SELECT 1` below
/// (which opens THE connection) runs before the counter is reset, and
/// `max_connections(1)` means every later checkout is a checkout of that one
/// idle connection — counted exactly once, in order.
/// `acquire_timeout` is deliberately short: nothing on a request path holds
/// a connection across another checkout, so if a test ever did it would fail
/// loudly with a pool timeout instead of waiting forever.
pub async fn spin_up_with_env_counting_acquires(
    overrides: &HashMap<String, String>,
) -> (TestApp, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    let (mut state, harness_pool) = build_state_and_pool(overrides, DEAD_LLM_URL).await;
    let counter = Arc::new(AtomicUsize::new(0));
    let measured = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .before_acquire({
            let counter = Arc::clone(&counter);
            move |_conn, _meta| {
                let counter = Arc::clone(&counter);
                Box::pin(async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Ok(true)
                })
            }
        })
        .connect(&state.config.database_url)
        .await
        .expect("connect the counted pool to the fresh per-test database");
    // Open THE connection now, so the physical connect is not the first
    // thing a measured request does, then start counting from zero.
    sqlx::query("SELECT 1")
        .execute(&measured)
        .await
        .expect("warm the counted pool");
    counter.store(0, Ordering::SeqCst);

    state.pg = Some(Arc::new(measured));
    let router = lakehouse_api::routes::router(state);
    (
        TestApp {
            router,
            pool: harness_pool,
        },
        counter,
    )
}

/// Shared by [`spin_up_with_env`] and
/// [`spin_up_with_connector_secret_resolver`]: provisions a fresh,
/// migrated, isolated database and builds the [`AppState`] over it, but
/// stops short of building the router — the one step a caller needs to
/// intercept in order to mutate `AppState` before routes are wired to it.
///
/// `dead_llm_url` says whether the harness's dead `LLM_URL` is in the
/// deployment's environment: with it there, the environment owns the
/// assistant's provider address (`ADM-9 review R4`), and without it the
/// console owns it.
async fn build_state_and_pool(
    overrides: &HashMap<String, String>,
    dead_llm_url: bool,
) -> (AppState, PgPool) {
    let base_url = lakehouse_test_support::database_url();
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&base_url)
        .await
        .expect("connect to the test-support admin database");

    let db_name = format!("lakehouse_api_test_{}", Uuid::new_v4().simple());
    // safe: `db_name` is this helper's own prefix plus a lowercase-hex UUID;
    // no caller text reaches the statement.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        r#"CREATE DATABASE "{db_name}""#
    )))
    .execute(&admin_pool)
    .await
    .expect("create a fresh per-test database");
    admin_pool.close().await;

    let db_url = swap_database_name(&base_url, &db_name);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to the fresh per-test database");
    lakehouse_store::migrate(&pool)
        .await
        .expect("apply migrations to the fresh per-test database");

    let mut env = HashMap::new();
    env.insert("DATABASE_URL".to_owned(), db_url);
    env.insert("CH_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    env.insert("DAGSTER_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    if dead_llm_url {
        env.insert("LLM_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    }
    env.insert(
        "CONNECTOR_SECRETS_KEY".to_owned(),
        test_connector_secrets_key(),
    );
    for (k, v) in overrides {
        env.insert(k.clone(), v.clone());
    }
    // Deliberately NOT `APP_ENV=development`: keeps the session cookie's
    // `Secure` attribute on, matching the fail-closed production default —
    // these tests never rely on the cookie flowing over plaintext HTTP,
    // since `tower::ServiceExt::oneshot` never touches a real socket.
    let config = Config::from_map(&env).expect("a valid test Config");
    let mut state = AppState::new(config);
    // `SEC-22` D3: the child that reads an uploaded workbook or a Parquet
    // file is the binary Cargo built for these tests — the test binary
    // itself would run the test suite again, so the runner is pointed at
    // the real child.
    state.conversion_runner = state
        .conversion_runner
        .with_binary(std::path::PathBuf::from(env!(
            "CARGO_BIN_EXE_lakehouse-api"
        )));
    // OPS-4: the harness applied the migrations to this database before
    // building the state, which is what `main::bring_database_up` confirms
    // in production — so `GET /health` reports the database as usable here
    // rather than as "never checked".
    state.mark_database_ready();

    (state, pool)
}

/// A minimal but spec-valid Iceberg REST `LoadTableResult` for `bronze.<table>`:
/// format version 2, one schema, one snapshot. It is what
/// `lakehouse_iceberg::rest::load_table_summary` parses without an error, so
/// `routes::catalog_restore::table_exists` reads the table as there.
///
/// Hand-written from the shape the Iceberg REST spec requires, and proven by
/// `lakehouse-iceberg`'s own fixture of the same shape; a test-only stand-in,
/// never captured from a live catalog.
pub fn load_table_fixture(table: &str) -> String {
    format!(
        r#"{{
            "metadata-location": "s3://lakehouse-warehouse/bronze/{table}/metadata/00001.metadata.json",
            "metadata": {{
                "format-version": 2,
                "table-uuid": "9c12d441-03fe-4bb1-8e5c-b2e7c1a04a3d",
                "location": "s3://lakehouse-warehouse/bronze/{table}",
                "last-sequence-number": 1,
                "last-updated-ms": 1700000000000,
                "last-column-id": 1,
                "schemas": [
                    {{
                        "schema-id": 0,
                        "type": "struct",
                        "fields": [{{"id": 1, "name": "id", "required": true, "type": "long"}}]
                    }}
                ],
                "current-schema-id": 0,
                "partition-specs": [{{"spec-id": 0, "fields": []}}],
                "default-spec-id": 0,
                "last-partition-id": 999,
                "properties": {{}},
                "current-snapshot-id": 1,
                "snapshots": [
                    {{
                        "snapshot-id": 1,
                        "sequence-number": 1,
                        "timestamp-ms": 1700000000000,
                        "manifest-list": "s3://lakehouse-warehouse/bronze/{table}/metadata/snap-1.avro",
                        "summary": {{ "operation": "append", "total-records": "1" }}
                    }}
                ],
                "metadata-log": [],
                "sort-orders": [{{"order-id": 0, "fields": []}}],
                "default-sort-order-id": 0
            }}
        }}"#
    )
}

/// Mount the Iceberg REST catalog so that `bronze.<table>` exists: the
/// `/v1/config` every Iceberg client asks first, and the table's load
/// (`DATA-55` review R2 — the healing rule and the restore's mirror case need
/// the "the table is there" answer from the catalog the restore itself reads).
///
/// `server` is whatever the deployment's `LAKEKEEPER_CATALOG_URI` points at.
pub async fn mount_existing_iceberg_table(server: &wiremock::MockServer, table: &str) {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, ResponseTemplate};

    Mock::given(method("GET"))
        .and(path("/v1/config"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/namespaces/bronze/tables/{table}")))
        .respond_with(ResponseTemplate::new(200).set_body_string(load_table_fixture(table)))
        .mount(server)
        .await;
}

/// Swaps the database name in `postgres://user:pass@host:port/dbname` for
/// `new_db`, without pulling in a URL-parsing crate for one substitution.
fn swap_database_name(url: &str, new_db: &str) -> String {
    let (base, _old_db) = url.rsplit_once('/').expect("a postgres:// URL with a path");
    format!("{base}/{new_db}")
}

/// Mints a live session for the seeded `email` (from
/// `0002_seed_identity.sql` — see that migration for the full roster) and
/// returns the `Cookie` header value to send it as.
pub async fn session_cookie_for_seeded_user(pool: &PgPool, email: &str) -> String {
    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}"));
    session_cookie_for_user(pool, user_id).await
}

/// Mints a live session for an arbitrary `app_user.id` and returns the
/// `Cookie` header value to send it as.
pub async fn session_cookie_for_user(pool: &PgPool, user_id: Uuid) -> String {
    let token: Secret = lakehouse_auth::session::create_session(
        pool,
        user_id,
        lakehouse_auth::session::DEFAULT_SESSION_TTL,
        None,
        None,
        &[],
    )
    .await
    .expect("mint a session for a test principal");
    format!("lh_session={}", token.expose())
}

/// `SEC-50` slice 2 M11: the tenant that owns the shared catalog in the
/// tests that read it.
///
/// The seeded identity migration creates four tenants, so a deployment with
/// no `CATALOG_TENANT_ID` has no tenant that owns the shared catalog — and
/// every read of it is refused (`SEC-50` D2). A test that reads the shared
/// catalog names this tenant (in the environment it starts the app with)
/// and puts its reader in it.
pub const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";

/// Puts `user` in the catalog's own tenant, so a read of the shared
/// catalog is the read the test means (`SEC-50` slice 2 M11).
pub async fn in_the_catalog_tenant(pool: &PgPool, user: Uuid) {
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user)
    .bind(CATALOG_TENANT.parse::<Uuid>().expect("a seeded tenant id"))
    .execute(pool)
    .await
    .expect("add the membership");
}

/// `SEC-50` slice 2 M11: makes the deployment hold no tenant of the seeded
/// ones, so the tenant gate changes nothing (D2) and a test that is about
/// its own rule keeps it.
///
/// The seeded identity migration creates four tenants, so a deployment with
/// several of them has no tenant that owns the shared catalog unless it
/// names one (`CATALOG_TENANT_ID`) — and every read of the shared catalog
/// is then refused. A test that reads the shared catalog and is not about
/// the gate either names the catalog's tenant (and puts its reader in it,
/// see [`in_the_catalog_tenant`]) or makes the deployment a one-tenant one
/// instead: there is then no other tenant's table to reach, and nothing
/// changes.
///
/// The membership rows go first: `app_user_tenant.tenant_id` refuses the
/// deletion of a tenant that still has members.
pub async fn leave_no_seeded_tenants(pool: &PgPool) {
    for tenant in [
        "11111111-1111-4111-8111-000000000001",
        "11111111-1111-4111-8111-000000000002",
        "11111111-1111-4111-8111-000000000003",
        "11111111-1111-4111-8111-000000000004",
    ] {
        let tenant_id: Uuid = tenant.parse().expect("a seeded tenant id");
        sqlx::query("DELETE FROM app_user_tenant WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the memberships of the tenant that goes");
        sqlx::query("DELETE FROM tenant WHERE id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the tenant");
    }
}

/// Creates a fresh `app_user` holding a role that grants NO permissions at
/// all (`role.permissions = ''`), and returns its `app_user.id`.
///
/// This is the "authenticated, but must be denied by every
/// `Policy::RequiresPermission` entry" principal the authz matrix needs —
/// no seeded role is a clean stand-in for "holds zero permissions" (even
/// `Dashboard Viewer` holds `dashboard:read`), so the test fixture creates
/// one explicitly rather than repurposing a seeded role that happens, for
/// now, not to satisfy whichever permission a given test checks.
///
/// Returns the user id, not a session cookie: callers that send more than
/// one request (e.g. looping every `POLICY_TABLE` entry, which includes
/// `POST /api/auth/logout` — a request that revokes whatever session
/// cookie it is sent with) must mint a FRESH session per request via
/// [`session_cookie_for_user`] rather than reuse one cookie across many
/// requests.
pub async fn create_zero_permission_principal(pool: &PgPool) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, '', 'test: grants nothing')",
    )
    .bind(role_id)
    .bind(format!("Zero Perm {}", Uuid::new_v4()))
    .execute(pool)
    .await
    .expect("insert a zero-permission role");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Zero Perm Test User', $2, $3, 'active')",
    )
    .bind(user_id)
    .bind(format!("zero-perm-{user_id}"))
    .bind(format!("zero-perm-{user_id}@test.invalid"))
    .execute(pool)
    .await
    .expect("insert a zero-permission test user");

    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .expect("attach the zero-permission role to the test user");

    user_id
}

/// Creates a fresh `app_user` who may use the assistant (`assistant:use`)
/// and who holds `permissions` besides — the principal a test about what a
/// tool does *inside* `/api/ai/*` needs since `ADM-13` D5 put that permission
/// on every assistant route (before it, any signed-in person reached the
/// tools; now a tool's own gate is only reached by somebody who may use the
/// assistant at all).
///
/// `permissions` is a `role.permissions`-shaped string and may be empty: a
/// person whose only grant is the assistant is what a test proving "each tool
/// refuses somebody without *its* permission" wants, because the request has
/// to reach the tool for the tool's refusal to be the thing under test.
///
/// Not a change to [`create_principal_with_permissions`]: that helper's other
/// callers are tests about non-assistant surfaces (dashboards, connectors,
/// the identity routes), where holding `assistant:use` would make the
/// principal something the test does not mean.
pub async fn create_assistant_principal(pool: &PgPool, permissions: &str) -> Uuid {
    let permissions = if permissions.is_empty() {
        "assistant:use".to_owned()
    } else {
        format!("{permissions},assistant:use")
    };
    create_principal_with_permissions(pool, &permissions).await
}

/// Creates a fresh `app_user` holding a role that grants EXACTLY
/// `permissions` (a `role.permissions`-shaped string, e.g. `"ingest:read"`
/// or `"connector:manage"`) and returns its `app_user.id`.
///
/// Unlike [`create_zero_permission_principal`], this lets a test assert
/// the negative half of a two-permission split: no seeded role holds
/// `ingest:read` alone or `connector:manage` alone (the only seeded role
/// with either, Data Engineer, holds BOTH after
/// `0033_connector_ingest_spec.sql`'s grant), so a route whose GET and PUT
/// require different permissions needs a principal minted with exactly
/// one of the two to prove the split is real and not accidentally
/// satisfied by the other permission.
pub async fn create_principal_with_permissions(pool: &PgPool, permissions: &str) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test: single-permission principal')",
    )
    .bind(role_id)
    .bind(format!("Single Perm {}", Uuid::new_v4()))
    .bind(permissions)
    .execute(pool)
    .await
    .expect("insert a single-permission role");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, 'Single Perm Test User', $2, $3, 'active')",
    )
    .bind(user_id)
    .bind(format!("single-perm-{user_id}"))
    .bind(format!("single-perm-{user_id}@test.invalid"))
    .execute(pool)
    .await
    .expect("insert a single-permission test user");

    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .expect("attach the single-permission role to the test user");

    user_id
}
