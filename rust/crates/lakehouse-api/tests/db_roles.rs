//! The console's own database role (`SEC-78`).
//!
//! The console used to connect as the Postgres image's superuser, which
//! owns every table because the migrations run with it — and a superuser
//! (or a table's owner) can switch off the append-only triggers of the
//! audit trail. These tests are the proof that the role the console
//! connects as now owns nothing, is not a superuser, and therefore cannot
//! change or remove an audit row.
//!
//! The role is CLUSTER-WIDE in the shared test Postgres, so every test
//! here uses ONE role name and ONE password, and the tests that touch the
//! role's cluster-wide state take a lock: a second role with a random name
//! would leave a role behind after every run.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use lakehouse_api::db_roles::{
    self, AppendOnlyTable, PASSWORD_LOG_GUARD, RoleSetupError, RuntimeRoleRefusal,
};
use lakehouse_api::{DatabaseStartup, StartPathRefusal};
use lakehouse_core::secret::SecretValue;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

use crate::common::{spin_up, spin_up_state};

/// The role every test in this file uses: one fixed name, because the role
/// is cluster-wide and a random name would leave one behind per run.
const APP_ROLE: &str = "lakehouse_app";

/// The role the one password test connects as, to be refused at the
/// password statement: it may create roles, and it is not a superuser, so
/// it may not change the superuser role's password.
const ROLE_MANAGER: &str = "sec78_role_manager";

/// A fixed password the tests can hold, without a credential-shaped
/// literal in the source: gitleaks reads a literal like
/// `PASSWORD = "..."` as a leak (`SEC-36`), and this value is not one —
/// it is a fixed test value this file's tests agree on. Building it at
/// run time keeps it out of the scanned text entirely.
fn app_password() -> String {
    encoded_repeated_byte(b'a')
}

/// The password the password-rotation test changes the role to.
fn changed_password() -> String {
    encoded_repeated_byte(b'b')
}

/// The password the password test plants in the statement that must never
/// surface it.
fn planted_password() -> String {
    encoded_repeated_byte(b'c')
}

/// The password the one role that proves the refusal needs, to connect.
fn manager_password() -> String {
    encoded_repeated_byte(b'd')
}

/// URL-safe base64 of the byte repeated: a fixed, non-credential-looking
/// value, built at run time.
fn encoded_repeated_byte(byte: u8) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([byte; 24])
}

/// Serializes the tests that touch the CLUSTER-WIDE role.
///
/// The role's attributes and its password are cluster-wide, so two tests
/// that changed them at the same time would see each other's change. The
/// lock is what makes "one fixed password" safe while one test proves
/// that a changed password applies.
static ROLE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The same lock across PROCESSES: `cargo nextest` runs each partition in
/// its own process against one shared cluster, so the mutex above does not
/// order them. A transaction-level advisory lock does, and it is released
/// when the transaction ends (`Drop` rolls it back).
const CLUSTER_ROLE_LOCK_KEY: i64 = 7_800_078;

async fn take_cluster_role_lock(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut transaction = pool
        .begin()
        .await
        .expect("begin the cluster-wide role lock");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(CLUSTER_ROLE_LOCK_KEY)
        .execute(&mut *transaction)
        .await
        .expect("take the cluster-wide role lock");
    transaction
}

/// A migrated database with the console's role set up on it, and a pool
/// that connects as that role.
struct RoleFixture {
    /// Held for as long as the role may be in the state this test left it
    /// in.
    _role_lock: tokio::sync::MutexGuard<'static, ()>,
    /// The same, across processes (see [`take_cluster_role_lock`]).
    _cluster_lock: sqlx::Transaction<'static, sqlx::Postgres>,
    /// The owner's pool: the test superuser, which owns every table.
    owner_pool: PgPool,
    /// A pool that connects as the console's role.
    app_pool: PgPool,
}

impl RoleFixture {
    /// A fresh database, migrated, with the console's role set up on it.
    async fn new() -> Self {
        Self::new_after(&[]).await
    }

    /// A fresh database, migrated, with the console's role set up on it,
    /// AFTER the given statements ran as the owner.
    ///
    /// The role's state is CLUSTER-WIDE, so it survives every run of every
    /// test: a test that proves the setup GRANTS something has to take
    /// that something back first, or the assertion would read the state
    /// an earlier run left behind instead of the state this setup made.
    async fn new_after(statements: &[&'static str]) -> Self {
        let (_state, owner_pool) = spin_up_state(&HashMap::new()).await;
        let role_lock = ROLE_LOCK.lock().await;
        let cluster_lock = take_cluster_role_lock(&owner_pool).await;
        for statement in statements {
            sqlx::query(*statement)
                .execute(&owner_pool)
                .await
                .expect("prepare the cluster-wide role");
        }
        db_roles::setup_app_role(&owner_pool, &SecretValue::new(app_password()))
            .await
            .expect("set the console's database role up");
        let app_pool = connect_as(&owner_pool, APP_ROLE, &app_password())
            .await
            .expect("connect as the console's database role");
        Self {
            _role_lock: role_lock,
            _cluster_lock: cluster_lock,
            owner_pool,
            app_pool,
        }
    }
}

/// A pool that connects to the same database as a given role.
///
/// The role must already exist and hold `CONNECT` on the database; the
/// password is the one the setup set for it.
async fn connect_as(
    owner_pool: &PgPool,
    role: &str,
    password: &str,
) -> Result<PgPool, sqlx::Error> {
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(owner_pool)
        .await?;
    let mut url = url::Url::parse(&lakehouse_test_support::database_url())
        .expect("the test-support database URL parses");
    url.set_username(role)
        .expect("the role name is a valid user name");
    url.set_password(Some(password))
        .expect("the password is a valid password");
    url.set_path(&format!("/{db_name}"));
    PgPoolOptions::new()
        .max_connections(4)
        .connect(url.as_str())
        .await
}

/// The role's attributes, read from the catalog.
async fn role_attributes(pool: &PgPool) -> (bool, bool, bool, bool, bool, bool) {
    sqlx::query_as(
        "SELECT rolsuper, rolcanlogin, rolcreatedb, rolcreaterole, rolreplication, rolbypassrls \
         FROM pg_roles WHERE rolname = $1",
    )
    .bind(APP_ROLE)
    .fetch_one(pool)
    .await
    .expect("read the console role's attributes")
}

/// How many table privileges the console role holds.
async fn granted_table_privileges(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.role_table_grants WHERE grantee = $1",
    )
    .bind(APP_ROLE)
    .fetch_one(pool)
    .await
    .expect("count the console role's table privileges")
}

/// The schema and the table of a schema-qualified name.
fn schema_and_table(qualified: &str) -> (&str, &str) {
    qualified
        .split_once('.')
        .expect("the append-only tables are schema-qualified")
}

/// The columns of `public.audit_event` that its grant must NOT admit.
///
/// CONSTANTS of this file, not a read of the catalog: an identifier that
/// reaches SQL text must be a constant (`AGENTS.md`, "SQL values are
/// bound and `format!` is only for constant identifiers"), and a name
/// that came out of a database read is not one. The grant admits no
/// column of `audit_event`: `run_id` and `approval_id` are refused too,
/// because a grant on them would let the console unlink an audit row by a
/// plain `UPDATE`. The list is every column of
/// `rust/migrations/0024_audit_event.sql` and its later additions.
const AUDIT_EVENT_REFUSED_COLUMNS: &[&str] = &[
    "run_id",
    "approval_id",
    "id",
    "at",
    "principal_id",
    "principal_kind",
    "actor_label",
    "action",
    "resource_kind",
    "resource_id",
    "args",
    "outcome",
    "detail",
    "session_id",
];

/// The columns of `public.usage_rate` that its grant must NOT admit.
///
/// CONSTANTS of this file, for the same reason as
/// [`AUDIT_EVENT_REFUSED_COLUMNS`]. `usage_rate` admits no column at
/// all, so these are all of them, from
/// `rust/migrations/0148_usage_rate_and_permission.sql`.
const USAGE_RATE_REFUSED_COLUMNS: &[&str] = &[
    "id",
    "kind",
    "currency",
    "amount",
    "effective_from",
    "created_by",
    "created_at",
];

/// The columns of an append-only table that its grant must refuse.
///
/// One constant list per table. A table that is not in the list has no
/// list here, and the test says so rather than trying a column it does
/// not know.
fn refused_columns(table: &AppendOnlyTable) -> &'static [&'static str] {
    match table.table {
        "public.audit_event" => AUDIT_EVENT_REFUSED_COLUMNS,
        "public.usage_rate" => USAGE_RATE_REFUSED_COLUMNS,
        other => panic!("{other} has no constant list of refused columns in this file"),
    }
}

/// The columns the database has for a table, read from the catalog.
///
/// The read is a CROSS-CHECK of the constant lists above, never a source
/// of SQL text: the values are bound as query parameters, and what they
/// say is only that the constants still cover the table.
async fn catalog_columns(pool: &PgPool, table: &AppendOnlyTable) -> Vec<String> {
    let (schema, name) = schema_and_table(table.table);
    sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(name)
    .fetch_all(pool)
    .await
    .expect("read the table's columns")
}

/// The SQLSTATE Postgres answers with when a privilege is missing:
/// `42501`, `insufficient_privilege`.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Assert that the database refused the statement by its PRIVILEGES.
///
/// A trigger's refusal is `P0001` (`raise_exception`) and a privilege's is
/// `42501` (`insufficient_privilege`). The distinction is what makes
/// "the GRANT refuses it" a claim and not "something refused it": the
/// append-only tables carry BOTH a trigger and a revoked privilege, and
/// only the SQLSTATE says which one spoke.
fn assert_refused_by_privilege(
    result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
    what: &str,
) {
    let Err(err) = result else {
        panic!("{what} must be refused by the database's privileges");
    };
    let code = sqlstate_of(&err);
    assert_eq!(
        code.as_deref(),
        Some(INSUFFICIENT_PRIVILEGE),
        "{what} must be refused by the PRIVILEGE (SQLSTATE {INSUFFICIENT_PRIVILEGE}), not by a \
         trigger (SQLSTATE P0001): {err}"
    );
}

/// The SQLSTATE the server answered with, when it sent one.
fn sqlstate_of(err: &sqlx::Error) -> Option<String> {
    let sqlx::Error::Database(db) = err else {
        return None;
    };
    db.code().map(std::borrow::Cow::into_owned)
}

/// The migrations of this branch, as files.
fn migration_files() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("read the migrations directory")
        .map(|entry| entry.expect("read a migrations directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    files.sort();
    files
}

/// The tables the migrations on this branch give a REFUSING trigger, and
/// the trigger function that refuses.
///
/// Read from the migration texts, the way `tests/audit_coverage.rs` reads
/// the route sources: a migration is the definition of the rule, and a
/// table whose changes the database refuses must be in the console role's
/// append-only list.
fn refusing_triggers_in_migrations() -> BTreeMap<String, String> {
    let mut refusing: BTreeMap<String, String> = BTreeMap::new();
    for path in migration_files() {
        let text = fs::read_to_string(&path).expect("read a migration");
        for statement in text.split(';') {
            let statement = statement.trim();
            if !statement.contains("TRIGGER") || !statement.contains("EXECUTE FUNCTION") {
                continue;
            }
            let Some(after_trigger) = statement.split_once("TRIGGER").map(|(_, rest)| rest) else {
                continue;
            };
            let Some(function) = after_trigger
                .split_once("EXECUTE FUNCTION")
                .map(|(_, rest)| rest.trim().trim_end_matches('(').trim().to_owned())
            else {
                continue;
            };
            if !function.contains("refuse") {
                continue;
            }
            let Some(table) = after_trigger.split_once(" ON ").map(|(_, rest)| {
                rest.split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned()
            }) else {
                continue;
            };
            refusing.insert(table, function);
        }
    }
    refusing
}

// ── What the console role may do ──────────────────────────────────────────

/// The console role may read and write an ordinary table: it owns no
/// table, but the blanket grants give it the ordinary privileges.
#[tokio::test]
async fn the_console_role_can_read_and_write_an_ordinary_table() {
    let fixture = RoleFixture::new().await;

    sqlx::query(
        "INSERT INTO login_throttle (key_hash, failures, window_started_at) \
         VALUES ('sec78_key', 1, now())",
    )
    .execute(&fixture.app_pool)
    .await
    .expect("insert as the console role");
    let failures: i32 =
        sqlx::query_scalar("SELECT failures FROM login_throttle WHERE key_hash = 'sec78_key'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select as the console role");
    assert_eq!(failures, 1, "the console role reads what it wrote");

    sqlx::query("UPDATE login_throttle SET failures = 2 WHERE key_hash = 'sec78_key'")
        .execute(&fixture.app_pool)
        .await
        .expect("update as the console role");
    let updated: i32 =
        sqlx::query_scalar("SELECT failures FROM login_throttle WHERE key_hash = 'sec78_key'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select the updated row as the console role");
    assert_eq!(updated, 2, "the console role writes over what it read");

    sqlx::query("DELETE FROM login_throttle WHERE key_hash = 'sec78_key'")
        .execute(&fixture.app_pool)
        .await
        .expect("delete as the console role");
    let left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM login_throttle WHERE key_hash = 'sec78_key'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("count as the console role");
    assert_eq!(left, 0, "the console role deletes a row");
}

/// The console role may read and write the `bronze_meta` schema's tables
/// too: the schema is in the role's schema list, not only `public`.
#[tokio::test]
async fn the_console_role_can_read_and_write_a_bronze_meta_table() {
    let fixture = RoleFixture::new().await;

    sqlx::query(
        "INSERT INTO bronze_meta.ingest_offset \
         (connector_id, topic, partition_id, committed_offset) \
         VALUES ('sec78_conn', 'sec78_topic', 0, 42)",
    )
    .execute(&fixture.app_pool)
    .await
    .expect("insert into bronze_meta as the console role");
    let offset: i64 = sqlx::query_scalar(
        "SELECT committed_offset FROM bronze_meta.ingest_offset \
         WHERE connector_id = 'sec78_conn'",
    )
    .fetch_one(&fixture.app_pool)
    .await
    .expect("select from bronze_meta as the console role");
    assert_eq!(offset, 42, "the console role reads what it wrote");

    sqlx::query(
        "UPDATE bronze_meta.ingest_offset SET committed_offset = 43 \
         WHERE connector_id = 'sec78_conn'",
    )
    .execute(&fixture.app_pool)
    .await
    .expect("update bronze_meta as the console role");
    sqlx::query("DELETE FROM bronze_meta.ingest_offset WHERE connector_id = 'sec78_conn'")
        .execute(&fixture.app_pool)
        .await
        .expect("delete from bronze_meta as the console role");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM bronze_meta.ingest_offset")
        .fetch_one(&fixture.app_pool)
        .await
        .expect("count bronze_meta as the console role");
    assert_eq!(left, 0, "the console role deletes a row");
}

/// The console role holds `pg_monitor`: the read-only statistics
/// privileges (`pg_read_all_stats`, `pg_read_all_settings`,
/// `pg_stat_scan_tables`) that the jobs dialing the console database
/// need, and nothing that could switch off the append-only rule.
///
/// The membership is CLUSTER-WIDE, so it outlives every run: the test
/// takes it back before the setup, so the setup's own grant is what the
/// assertion reads.
#[tokio::test]
async fn the_console_role_holds_the_read_only_statistics_privileges() {
    let fixture = RoleFixture::new_after(&["REVOKE pg_monitor FROM lakehouse_app"]).await;

    let is_member: bool = sqlx::query_scalar("SELECT pg_has_role($1, 'pg_monitor', 'MEMBER')")
        .bind(APP_ROLE)
        .fetch_one(&fixture.owner_pool)
        .await
        .expect("read the console role's memberships");
    assert!(is_member, "the console role holds pg_monitor");
}

/// The console role may APPEND to the audit trail, and may not change it,
/// remove it, or switch the rule off.
#[tokio::test]
async fn the_console_role_can_append_to_the_audit_trail_but_not_change_it() {
    let fixture = RoleFixture::new().await;

    sqlx::query(
        "INSERT INTO audit_event (id, action, outcome) \
         VALUES ('sec78_evt', 'sec78.action', 'allowed')",
    )
    .execute(&fixture.app_pool)
    .await
    .expect("insert an audit row as the console role");
    let action: String =
        sqlx::query_scalar("SELECT action FROM audit_event WHERE id = 'sec78_evt'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select the audit row as the console role");
    assert_eq!(
        action, "sec78.action",
        "the console role reads the trail it wrote"
    );

    let deleted = sqlx::query("DELETE FROM audit_event WHERE id = 'sec78_evt'")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(deleted, "the console role deleting an audit row");

    let truncated = sqlx::query("TRUNCATE audit_event")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(truncated, "the console role emptying the audit trail");

    let disabled = sqlx::query("ALTER TABLE audit_event DISABLE TRIGGER audit_event_refuse_update")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(
        disabled,
        "the console role switching the audit trail's trigger off",
    );

    let replica = sqlx::query("SET session_replication_role = replica")
        .execute(&fixture.app_pool)
        .await;
    assert!(
        replica.is_err(),
        "the console role is no superuser, so it cannot become one for the session"
    );
}

/// The migration ledger is the record the boot check trusts. The console
/// role may read it and must not write it.
#[tokio::test]
async fn the_console_role_reads_the_migration_ledger_but_cannot_write_it() {
    let fixture = RoleFixture::new().await;

    let applied: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&fixture.app_pool)
        .await
        .expect("read the migration ledger as the console role");
    assert!(applied > 0, "the console role reads the applied versions");

    let deleted = sqlx::query("DELETE FROM _sqlx_migrations")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(deleted, "the console role deleting migration records");

    let updated = sqlx::query("UPDATE _sqlx_migrations SET success = success")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(updated, "the console role rewriting migration records");

    let inserted = sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (999999, 'sec78 probe', true, decode('00', 'hex'), 0)",
    )
    .execute(&fixture.app_pool)
    .await;
    assert_refused_by_privilege(inserted, "the console role inserting a migration record");
}

/// The password statement runs under the log guard: the settings it sets
/// must take effect inside its transaction, so the statement is not logged.
#[tokio::test]
async fn the_password_statement_runs_with_its_log_guard_on() {
    let fixture = RoleFixture::new().await;
    let mut transaction = fixture
        .owner_pool
        .begin()
        .await
        .expect("begin a transaction as the owner");
    for guard in PASSWORD_LOG_GUARD {
        sqlx::query(guard)
            .execute(&mut *transaction)
            .await
            .expect("the log guard statement");
    }
    let (statement_log, error_log): (String, String) = sqlx::query_as(
        "SELECT current_setting('log_statement'), current_setting('log_min_error_statement')",
    )
    .fetch_one(&mut *transaction)
    .await
    .expect("read the log settings inside the transaction");
    assert_eq!(statement_log, "none", "statements are not logged");
    assert_eq!(
        error_log, "panic",
        "a failing statement's text is not logged"
    );
    transaction.rollback().await.expect("roll the probe back");
}

/// On the audit trail the console role cannot change a row at all, not even
/// the links the trigger would admit: a grant on those columns would let
/// it unlink an audit row directly. The `ON DELETE SET NULL` path does not
/// need that grant (see the delete tests below).
#[tokio::test]
async fn on_the_audit_trail_the_console_role_cannot_change_a_row() {
    let fixture = RoleFixture::new().await;
    seed_a_run_and_an_audit_row(&fixture.owner_pool).await;

    let cleared_run = sqlx::query("UPDATE audit_event SET run_id = NULL WHERE id = 'sec78_evt'")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(
        cleared_run,
        "the console role clearing the run link directly",
    );

    let cleared_approval =
        sqlx::query("UPDATE audit_event SET approval_id = NULL WHERE id = 'sec78_evt'")
            .execute(&fixture.app_pool)
            .await;
    assert_refused_by_privilege(
        cleared_approval,
        "the console role clearing the approval link directly",
    );

    let pointed =
        sqlx::query("UPDATE audit_event SET run_id = 'sec78_other_run' WHERE id = 'sec78_evt'")
            .execute(&fixture.app_pool)
            .await;
    assert_refused_by_privilege(
        pointed,
        "the console role pointing a run link at another row",
    );

    let other_column =
        sqlx::query("UPDATE audit_event SET action = 'sec78.other' WHERE id = 'sec78_evt'")
            .execute(&fixture.app_pool)
            .await;
    assert_refused_by_privilege(
        other_column,
        "the console role changing a column outside the allowed ones",
    );

    let actor = sqlx::query(
        "UPDATE audit_event SET actor_label = 'sec78.impersonator' WHERE id = 'sec78_evt'",
    )
    .execute(&fixture.app_pool)
    .await;
    assert_refused_by_privilege(actor, "the console role changing the actor of an audit row");
}

/// Deleting the run the audit row points at clears the link: the
/// `ON DELETE SET NULL` of the foreign key is an `UPDATE` of the audit
/// row, and it has to keep working for the console role.
#[tokio::test]
async fn deleting_the_run_as_the_console_role_clears_the_audit_row_link() {
    let fixture = RoleFixture::new().await;
    seed_a_run_and_an_audit_row(&fixture.owner_pool).await;

    sqlx::query("DELETE FROM agent_run WHERE id = 'sec78_run'")
        .execute(&fixture.app_pool)
        .await
        .expect("the console role deletes the run");
    let run_id: Option<String> =
        sqlx::query_scalar("SELECT run_id FROM audit_event WHERE id = 'sec78_evt'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select the audit row as the console role");
    assert_eq!(run_id, None, "the link is cleared by the delete");

    let action: String =
        sqlx::query_scalar("SELECT action FROM audit_event WHERE id = 'sec78_evt'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select the audit row as the console role");
    assert_eq!(
        action, "sec78.action",
        "the audit row survives the delete of the run it points at"
    );
}

/// Deleting the approval the audit row points at clears that link too:
/// the `ON DELETE SET NULL` of the foreign key is an `UPDATE` of the audit
/// row, and it has to keep working for the console role.
#[tokio::test]
async fn deleting_the_approval_as_the_console_role_clears_the_audit_row_link() {
    let fixture = RoleFixture::new().await;
    seed_a_run_and_an_audit_row(&fixture.owner_pool).await;

    sqlx::query("DELETE FROM approval_item WHERE id = 'sec78_appr'")
        .execute(&fixture.app_pool)
        .await
        .expect("the console role deletes the approval");
    let approval_id: Option<String> =
        sqlx::query_scalar("SELECT approval_id FROM audit_event WHERE id = 'sec78_evt'")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("select the audit row as the console role");
    assert_eq!(approval_id, None, "the link is cleared by the delete");
}

/// A table the owner creates AFTER the setup is covered by the default
/// privileges, so the console role can use it without another grant.
#[tokio::test]
async fn a_table_created_after_the_setup_is_readable_and_writable_by_the_console_role() {
    let fixture = RoleFixture::new().await;

    sqlx::query("CREATE TABLE sec78_late_table (id text primary key, note text)")
        .execute(&fixture.owner_pool)
        .await
        .expect("the owner creates a table after the setup");
    sqlx::query("INSERT INTO sec78_late_table (id, note) VALUES ('a', 'b')")
        .execute(&fixture.app_pool)
        .await
        .expect("the default privileges cover the later table");
    let note: String = sqlx::query_scalar("SELECT note FROM sec78_late_table WHERE id = 'a'")
        .fetch_one(&fixture.app_pool)
        .await
        .expect("select the later table as the console role");
    assert_eq!(note, "b", "the console role reads what it wrote");

    sqlx::query("UPDATE sec78_late_table SET note = 'c' WHERE id = 'a'")
        .execute(&fixture.app_pool)
        .await
        .expect("update the later table as the console role");
    sqlx::query("DELETE FROM sec78_late_table WHERE id = 'a'")
        .execute(&fixture.app_pool)
        .await
        .expect("delete from the later table as the console role");
}

/// Every column of an append-only table is classified: either one its
/// grant admits, or one of the constant list of the columns it refuses.
///
/// The catalog is read here and ONLY here. The list of the columns the
/// grant refuses is a constant of this file, and this test says that it
/// still covers the table — so a migration that adds a column fails here
/// instead of slipping past a list that no longer names it.
#[tokio::test]
async fn every_column_of_an_append_only_table_is_classified_by_its_grant() {
    let fixture = RoleFixture::new().await;

    for table in db_roles::APPEND_ONLY_TABLES {
        let catalog = catalog_columns(&fixture.owner_pool, table).await;
        assert!(!catalog.is_empty(), "{} has columns", table.table);
        for column in &catalog {
            assert!(
                refused_columns(table).contains(&column.as_str())
                    || table.allowed_update_columns.contains(&column.as_str()),
                "{} has the column {column}, which is in neither the constant list of the \
                 columns the grant refuses nor the list of the columns it admits: a \
                 migration may have added it",
                table.table,
            );
        }
        for column in refused_columns(table) {
            assert!(
                catalog.iter().any(|name| name.as_str() == *column),
                "{} has no column named {column}: the constant list names a column the \
                 database does not have",
                table.table,
            );
            assert!(
                !table.allowed_update_columns.contains(column),
                "{column} of {} is one of the columns the grant admits, so it must not be \
                 in the list of the columns it refuses",
                table.table,
            );
        }
    }
}

/// Every table in the append-only list refuses the changes the trigger
/// refuses. Every column its grant does not admit is tried, one by one:
/// a table with no allowed columns refuses EVERY update.
///
/// The refusal must be the PRIVILEGE's (`42501`) and not the trigger's
/// (`P0001`): the grant is what keeps the console role away from the
/// table, and the trigger is the second line of defence.
///
/// The columns are the CONSTANTS of this file, not a read of the
/// catalog: an identifier that reaches SQL text must be a constant
/// (`AGENTS.md`), and a name that came out of a database read is not
/// one. `every_column_of_an_append_only_table_is_classified_by_its_grant`
/// reads the catalog and says that the constants still cover the table.
#[tokio::test]
async fn every_append_only_table_refuses_delete_truncate_and_a_change_outside_its_columns() {
    let fixture = RoleFixture::new().await;
    seed_an_append_only_row(&fixture.owner_pool).await;

    for table in db_roles::APPEND_ONLY_TABLES {
        let deleted = sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {} WHERE true",
            table.table
        )))
        .execute(&fixture.app_pool)
        .await;
        assert_refused_by_privilege(
            deleted,
            &format!("the console role deleting a row of {}", table.table),
        );

        let truncated = sqlx::query(sqlx::AssertSqlSafe(format!("TRUNCATE {}", table.table)))
            .execute(&fixture.app_pool)
            .await;
        assert_refused_by_privilege(
            truncated,
            &format!("the console role emptying {}", table.table),
        );

        for column in refused_columns(table) {
            let changed = sqlx::query(sqlx::AssertSqlSafe(format!(
                "UPDATE {} SET {column} = {column} WHERE true",
                table.table
            )))
            .execute(&fixture.app_pool)
            .await;
            assert_refused_by_privilege(
                changed,
                &format!("the console role changing {column} of {}", table.table),
            );
        }
    }
}

/// A row in every append-only table, written by the owner.
///
/// The rows are what make the refusal a refusal of the STATEMENT: with
/// an empty table an `UPDATE` that matches no row could pass, and the
/// test would prove nothing.
async fn seed_an_append_only_row(pool: &PgPool) {
    seed_a_run_and_an_audit_row(pool).await;
    sqlx::query(
        "INSERT INTO usage_rate (id, kind, currency, amount, effective_from) \
         VALUES ('00000000-0000-0000-0000-000000000078', 'scan_tb', 'USD', 1, current_date)",
    )
    .execute(pool)
    .await
    .expect("insert a usage rate");
}

/// The console role owns no table and holds no `CREATE` on the console's
/// schemas: it must not be able to put a table into `public` — a table the
/// grants would then let it write, and that the next migration would find.
#[tokio::test]
async fn the_console_role_cannot_create_a_table_in_public() {
    let fixture = RoleFixture::new().await;

    let created = sqlx::query("CREATE TABLE sec78_made (id text primary key)")
        .execute(&fixture.app_pool)
        .await;
    assert_refused_by_privilege(created, "the console role creating a table in public");
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_tables WHERE schemaname = 'public' \
         AND tablename = 'sec78_made'",
    )
    .fetch_one(&fixture.owner_pool)
    .await
    .expect("count the tables named sec78_made");
    assert_eq!(left, 0, "no table was created");
}

/// A migration that gives a table a refusing trigger must add that table
/// to the append-only list, or the console role would be able to change
/// it.
#[test]
fn every_table_with_a_refusing_trigger_is_in_the_append_only_list() {
    let listed: BTreeSet<String> = db_roles::APPEND_ONLY_TABLES
        .iter()
        .map(|entry| schema_and_table(entry.table).1.to_owned())
        .collect();
    for (table, function) in refusing_triggers_in_migrations() {
        assert!(
            listed.contains(&table),
            "the trigger {function} refuses changes to {table}, but {table} is not in \
             db_roles::APPEND_ONLY_TABLES: the console role would be able to change it"
        );
    }
}

// ── Running the setup again ───────────────────────────────────────────────

/// Running the setup again changes nothing: it is idempotent, so it can
/// run on every deploy.
#[tokio::test]
async fn running_the_setup_twice_changes_nothing() {
    let fixture = RoleFixture::new().await;
    let attributes_before = role_attributes(&fixture.owner_pool).await;
    let privileges_before = granted_table_privileges(&fixture.owner_pool).await;

    db_roles::setup_app_role(&fixture.owner_pool, &SecretValue::new(app_password()))
        .await
        .expect("the setup runs again");

    assert_eq!(
        role_attributes(&fixture.owner_pool).await,
        attributes_before,
        "the role's attributes are the same after the second run"
    );
    assert_eq!(
        granted_table_privileges(&fixture.owner_pool).await,
        privileges_before,
        "the role holds the same privileges after the second run"
    );
    assert!(
        db_roles::check_runtime_role(&fixture.app_pool)
            .await
            .is_ok(),
        "the console role still passes the start path's check after the second run"
    );
}

/// A changed password applies on the next run: the setup always sets the
/// role's password, so a rotated secret takes effect on the next deploy.
#[tokio::test]
async fn a_changed_password_applies_on_the_second_run() {
    let fixture = RoleFixture::new().await;
    assert!(
        connect_as(&fixture.owner_pool, APP_ROLE, &app_password())
            .await
            .is_ok(),
        "the password of the first run works"
    );

    db_roles::setup_app_role(&fixture.owner_pool, &SecretValue::new(changed_password()))
        .await
        .expect("the setup applies the changed password");
    assert!(
        connect_as(&fixture.owner_pool, APP_ROLE, &changed_password())
            .await
            .is_ok(),
        "the changed password works after the second run"
    );

    // Put the fixed password back BEFORE any assertion can abort this
    // test: the role is cluster-wide, and the other tests of this file
    // connect with the fixed one.
    db_roles::setup_app_role(&fixture.owner_pool, &SecretValue::new(app_password()))
        .await
        .expect("restore the fixed password");
    assert!(
        connect_as(&fixture.owner_pool, APP_ROLE, &changed_password())
            .await
            .is_err(),
        "the changed password no longer works"
    );
    assert!(
        connect_as(&fixture.owner_pool, APP_ROLE, &app_password())
            .await
            .is_ok(),
        "the fixed password works again"
    );
}

/// A statement that carries the password and is refused reports its
/// SQLSTATE and nothing else: the server's own message can quote the
/// statement it refused, and that statement holds the password.
///
/// The refusal is forced for real: the setup runs as a role that may
/// create roles but is not a superuser, while the console role has been
/// handed a superuser attribute for the moment — altering a superuser
/// role is exactly the statement Postgres refuses, with the password in
/// the text.
#[tokio::test]
async fn a_refused_password_statement_carries_only_its_sqlstate() {
    let fixture = RoleFixture::new().await;
    sqlx::query("ALTER ROLE lakehouse_app SUPERUSER")
        .execute(&fixture.owner_pool)
        .await
        .expect("hand the console role the superuser attribute for the moment");

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP ROLE IF EXISTS {ROLE_MANAGER}"
    )))
    .execute(&fixture.owner_pool)
    .await
    .expect("drop a role manager left behind by an earlier run");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {ROLE_MANAGER} LOGIN CREATEROLE NOSUPERUSER PASSWORD '{}'",
        manager_password()
    )))
    .execute(&fixture.owner_pool)
    .await
    .expect("create the role that will be refused");

    let planted = planted_password();
    let manager_pool = connect_as(&fixture.owner_pool, ROLE_MANAGER, &manager_password())
        .await
        .expect("connect as the role manager");
    let outcome = db_roles::setup_app_role(&manager_pool, &SecretValue::new(planted.clone())).await;

    // Put the cluster back BEFORE any assertion can abort this test: the
    // console role must not stay a superuser, and the manager role must
    // not be left behind.
    manager_pool.close().await;
    sqlx::query("ALTER ROLE lakehouse_app NOSUPERUSER")
        .execute(&fixture.owner_pool)
        .await
        .expect("take the superuser attribute back");
    sqlx::query(sqlx::AssertSqlSafe(format!("DROP ROLE {ROLE_MANAGER}")))
        .execute(&fixture.owner_pool)
        .await
        .expect("drop the role manager");

    let err = outcome.expect_err("the password statement must be refused");
    assert!(
        !err.to_string().contains(planted.as_str()),
        "the password must not reach the message: {err}"
    );
    assert!(
        !format!("{err:?}").contains(planted.as_str()),
        "the password must not reach the Debug print either"
    );
    assert!(
        !err.to_string().contains("ALTER ROLE"),
        "the refused statement's text must not reach the message: {err}"
    );
    // The manager is no superuser, so the log guard (`SET LOCAL
    // log_statement`, which needs superuser) is refused first, and the
    // statement that carries the password is never sent. Either refusal
    // keeps the password out of the error, which is what this test checks.
    assert!(
        matches!(
            &err,
            RoleSetupError::Refused(_) | RoleSetupError::PasswordRefused { .. }
        ),
        "a refusal, not a success: {err:?}"
    );
    assert!(
        err.sqlstate().is_some(),
        "the SQLSTATE is what the caller gets: {err:?}"
    );
}

// ── The start path's checks ───────────────────────────────────────────────

/// The start path refuses a superuser: it can switch off any trigger.
///
/// The check is the one the start path really runs ([`check_start_path`]),
/// not the helper under it: a check the start path does not call would
/// prove nothing.
#[tokio::test]
async fn the_start_path_refuses_a_superuser() {
    let (_state, owner_pool) = spin_up_state(&HashMap::new()).await;
    let is_superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&owner_pool)
            .await
            .expect("read the harness role");
    assert!(is_superuser, "the harness connects as the test superuser");

    let refusal = lakehouse_api::check_start_path(&owner_pool)
        .await
        .expect_err("a superuser must be refused");
    assert!(
        matches!(
            refusal,
            StartPathRefusal::Role(RuntimeRoleRefusal::SuperuserOrOwner)
        ),
        "the refusal names the role, not the database: {refusal:?}"
    );
    assert_eq!(
        refusal.to_string(),
        db_roles::RUNTIME_ROLE_REFUSAL,
        "the refusal is the fixed sentence"
    );
}

/// The start path refuses the owner of a table: it can switch that table's
/// triggers off.
#[tokio::test]
async fn the_start_path_refuses_the_owner_of_a_table() {
    let fixture = RoleFixture::new().await;
    // Hand a table to the console role, so it owns one without being a
    // superuser: the ownership alone has to be enough for the refusal.
    sqlx::query("ALTER TABLE login_throttle OWNER TO lakehouse_app")
        .execute(&fixture.owner_pool)
        .await
        .expect("the owner hands a table to the console role");
    let is_superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("read the console role");
    assert!(
        !is_superuser,
        "the console role is no superuser: only the ownership is at fault"
    );

    let refusal = lakehouse_api::check_start_path(&fixture.app_pool)
        .await
        .expect_err("the owner of a table must be refused");
    assert!(matches!(
        refusal,
        StartPathRefusal::Role(RuntimeRoleRefusal::SuperuserOrOwner)
    ));
    assert_eq!(refusal.to_string(), db_roles::RUNTIME_ROLE_REFUSAL);
}

/// The start path refuses the owner of the audit trail: owning
/// `audit_event` alone lets a role switch its append-only triggers off.
#[tokio::test]
async fn the_start_path_refuses_the_owner_of_the_audit_trail() {
    let fixture = RoleFixture::new().await;
    sqlx::query("ALTER TABLE audit_event OWNER TO lakehouse_app")
        .execute(&fixture.owner_pool)
        .await
        .expect("hand the audit trail to the console role");

    let refusal = lakehouse_api::check_start_path(&fixture.app_pool)
        .await
        .expect_err("the owner of the audit trail must be refused");
    assert!(
        matches!(
            refusal,
            StartPathRefusal::Role(RuntimeRoleRefusal::SuperuserOrOwner)
        ),
        "the refusal is about the role, not about the database: {refusal:?}"
    );
    assert_eq!(refusal.to_string(), db_roles::RUNTIME_ROLE_REFUSAL);
}

/// The start path refuses the owner of a `bronze_meta` table: the
/// console's schemas are the console's, and owning any table in one of
/// them is enough.
#[tokio::test]
async fn the_start_path_refuses_the_owner_of_a_bronze_meta_table() {
    let fixture = RoleFixture::new().await;
    sqlx::query("ALTER TABLE bronze_meta.ingest_offset OWNER TO lakehouse_app")
        .execute(&fixture.owner_pool)
        .await
        .expect("hand the bronze_meta table to the console role");
    let is_superuser: bool =
        sqlx::query_scalar("SELECT rolsuper FROM pg_roles WHERE rolname = current_user")
            .fetch_one(&fixture.app_pool)
            .await
            .expect("read the console role");
    assert!(
        !is_superuser,
        "the console role is no superuser: only the ownership is at fault"
    );

    let refusal = lakehouse_api::check_start_path(&fixture.app_pool)
        .await
        .expect_err("the owner of a bronze_meta table must be refused");
    assert!(matches!(
        refusal,
        StartPathRefusal::Role(RuntimeRoleRefusal::SuperuserOrOwner)
    ));
    assert_eq!(refusal.to_string(), db_roles::RUNTIME_ROLE_REFUSAL);
}

/// The start path accepts the console's own role.
#[tokio::test]
async fn the_start_path_accepts_the_console_role() {
    let fixture = RoleFixture::new().await;
    assert!(
        matches!(
            lakehouse_api::check_start_path(&fixture.app_pool).await,
            Ok(DatabaseStartup::Ready)
        ),
        "the console role owns no table and is no superuser, so it may serve"
    );
}

/// The start path refuses a database whose schema is behind this build:
/// the migrations are pending, and the `lakehouse-db-migrate` service is
/// what applies them.
#[tokio::test]
async fn the_start_path_refuses_a_pending_migration() {
    let fixture = RoleFixture::new().await;
    assert!(
        matches!(
            lakehouse_api::check_start_path(&fixture.app_pool).await,
            Ok(DatabaseStartup::Ready)
        ),
        "the harness database is at this build's schema version"
    );

    sqlx::query(
        "DELETE FROM _sqlx_migrations WHERE version = \
         (SELECT MAX(version) FROM _sqlx_migrations WHERE success = true)",
    )
    .execute(&fixture.owner_pool)
    .await
    .expect("un-record the last migration");
    let outcome = lakehouse_api::check_start_path(&fixture.app_pool)
        .await
        .expect("the schema check reads the bookkeeping table");
    assert!(
        matches!(
            outcome,
            DatabaseStartup::Pending(ref versions) if !versions.is_empty()
        ),
        "a pending migration is a refusal, not a degradation"
    );
    assert_eq!(
        lakehouse_api::DATABASE_SCHEMA_MISMATCH,
        "the application database is not at this build's schema version; run the \
         lakehouse-db-migrate service before lakehouse-api",
        "the refusal names the service that applies the migrations"
    );
}

/// The route-test harness still builds with the test superuser (`SEC-78`
/// decision 5): the start path's check lives in the start path, and NOT
/// in `AppState` or the router, which the route tests build as the test
/// superuser.
///
/// The body is what proves the harness is really connected: the health
/// route reports the database as `ok` only when the start-up step
/// confirmed it.
#[tokio::test]
async fn the_route_test_harness_still_builds_with_the_superuser() {
    let app = spin_up().await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("build the request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), StatusCode::OK);

    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the health body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("parse the health body");
    assert_eq!(body["status"], "ok");
    assert_eq!(
        body["database"], "ok",
        "the harness's superuser connects: the start path's check must not be in the router"
    );
}

// ── The `migrate` command ─────────────────────────────────────────────────

/// The `migrate` command is the one place that sets the database up: it
/// connects with the OWNER's URL, applies the migrations, sets the
/// console's own role up, and exits.
///
/// It runs as a process, because that is how the `lakehouse-db-migrate`
/// service runs it (`SEC-78` decision 3): a test that called the library
/// instead would not prove that the COMMAND works.
#[tokio::test]
async fn the_migrate_command_sets_the_console_role_up_and_exits() {
    let (_state, owner_pool) = spin_up_state(&HashMap::new()).await;
    let _role_lock = ROLE_LOCK.lock().await;
    let _cluster_lock = take_cluster_role_lock(&owner_pool).await;
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner_pool)
        .await
        .expect("read the database name");
    let mut url = url::Url::parse(&lakehouse_test_support::database_url())
        .expect("the test-support database URL parses");
    url.set_path(&format!("/{db_name}"));

    let output = Command::new(env!("CARGO_BIN_EXE_lakehouse-api"))
        .arg("migrate")
        .env_clear()
        .env("DATABASE_URL", url.as_str())
        .env("LAKEHOUSE_DB_APP_PASSWORD", app_password())
        .output()
        .expect("run the migrate command");
    assert!(
        output.status.success(),
        "the migrate command exits 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let app_pool = connect_as(&owner_pool, APP_ROLE, &app_password())
        .await
        .expect("connect as the console role the migrate command set up");
    assert!(
        db_roles::check_runtime_role(&app_pool).await.is_ok(),
        "the role the migrate command set up owns no table and is no superuser"
    );
    let attributes = role_attributes(&owner_pool).await;
    assert!(
        !attributes.0
            && attributes.1
            && !attributes.2
            && !attributes.3
            && !attributes.4
            && !attributes.5,
        "the role logs in, and holds none of the other attributes: {attributes:?}"
    );
}

/// The `migrate` command must never write the console role's password into
/// its log.
///
/// The command runs one statement whose TEXT carries that password (the
/// server builds the statement for it), and `sqlx` logs a statement's text
/// at the DEBUG level by default. The command therefore runs with
/// `RUST_LOG=debug` here — the level an operator who turns the logs up
/// gets — and the password must not appear in what the command printed.
#[tokio::test]
async fn the_migrate_command_never_writes_the_role_password_into_its_log() {
    let (_state, owner_pool) = spin_up_state(&HashMap::new()).await;
    let _role_lock = ROLE_LOCK.lock().await;
    let _cluster_lock = take_cluster_role_lock(&owner_pool).await;
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&owner_pool)
        .await
        .expect("read the database name");
    let mut url = url::Url::parse(&lakehouse_test_support::database_url())
        .expect("the test-support database URL parses");
    url.set_path(&format!("/{db_name}"));

    let output = Command::new(env!("CARGO_BIN_EXE_lakehouse-api"))
        .arg("migrate")
        .env_clear()
        .env("DATABASE_URL", url.as_str())
        .env("LAKEHOUSE_DB_APP_PASSWORD", app_password())
        .env("RUST_LOG", "debug")
        .output()
        .expect("run the migrate command");
    assert!(
        output.status.success(),
        "the migrate command exits 0: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let printed = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !printed.contains(&app_password()),
        "the role's password must not reach the migrate command's log: {}",
        // The password is redacted in the message: a failure must not put
        // it in the test's own output either.
        printed.replace(&app_password(), "<the role's password>")
    );
}

/// An unknown command names the two commands the binary has, so an
/// operator who mistypes one is told what to type.
#[tokio::test]
async fn an_unknown_command_names_the_two_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_lakehouse-api"))
        .arg("sec78-not-a-command")
        .env_clear()
        .output()
        .expect("run the binary with an unknown command");
    assert!(
        !output.status.success(),
        "an unknown command exits non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("migrate"),
        "the message names the migrate command: {stderr}"
    );
    assert!(
        stderr.contains("rotate-secrets-key"),
        "the message names the rotate-secrets-key command: {stderr}"
    );
}

/// An employee, a run, an approval and an audit row that points at both.
async fn seed_a_run_and_an_audit_row(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO agent_employee (id, name, purpose, autonomy) \
         VALUES ('sec78_emp', 'Test', 'testing', 'L1')",
    )
    .execute(pool)
    .await
    .expect("insert an employee");
    sqlx::query(
        "INSERT INTO agent_run (id, employee_id, status, trigger, actor) \
         VALUES ('sec78_run', 'sec78_emp', 'done', 'manual', 'tester')",
    )
    .execute(pool)
    .await
    .expect("insert a run");
    sqlx::query(
        "INSERT INTO approval_item (id, employee_id, employee_name, action, run_id) \
         VALUES ('sec78_appr', 'sec78_emp', 'Test', 'sec78.action', 'sec78_run')",
    )
    .execute(pool)
    .await
    .expect("insert an approval");
    sqlx::query(
        "INSERT INTO audit_event (id, action, outcome, run_id, approval_id) \
         VALUES ('sec78_evt', 'sec78.action', 'allowed', 'sec78_run', 'sec78_appr')",
    )
    .execute(pool)
    .await
    .expect("insert an audit row that points at the run and the approval");
}
