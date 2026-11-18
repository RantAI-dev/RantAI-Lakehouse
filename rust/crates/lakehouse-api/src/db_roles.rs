//! The console's own database role (`SEC-78`).
//!
//! WHY A ROLE OF ITS OWN: the console used to connect as the Postgres
//! image's superuser, which owns every table because the migrations run
//! with it. A superuser, or the owner of a table, can switch off the
//! append-only triggers of `rust/migrations/0142_audit_event_append_only.sql`
//! — `ALTER TABLE audit_event DISABLE TRIGGER`, or
//! `SET session_replication_role = replica` — and change or delete the
//! audit trail. The role in this module owns nothing and is not a
//! superuser, so the console can do neither. The owner's role stays where
//! it is: the migrations still run with it.
//!
//! WHERE THE ROLE IS SET UP: `lakehouse-api migrate` runs the migrations
//! and then [`setup_app_role`], once per deploy, with the owner's
//! credentials. The API's own start path only READS the database
//! ([`check_runtime_role`]), so the API never holds the owner's password.
//!
//! WHY THE LISTS ARE HERE: the role's name, the schemas it may use and
//! the append-only tables it may not change are one list each, in one
//! module, so that a new schema or table is added in one place. A table a
//! later migration creates is covered by the default privileges
//! ([`setup_app_role`]); a new APPEND-ONLY table is not — it needs an
//! entry in [`APPEND_ONLY_TABLES`], because the blanket grants give the
//! role `UPDATE` on every table.

use std::fmt;

use lakehouse_core::secret::SecretValue;
use sqlx::PgPool;

/// The console's database role.
///
/// `LOGIN` and nothing else: not a superuser, no database creation, no
/// role creation, no replication, no `BYPASSRLS`, and it owns no table.
/// Those attributes are what keep the append-only triggers out of reach.
pub const APP_ROLE_NAME: &str = "lakehouse_app";

/// The schemas the console role may use.
///
/// `public` holds the console's own tables; `bronze_meta` holds the
/// ingest bookkeeping (`rust/migrations/0043_ingest_tier2_adapters.sql`).
/// A new schema needs an entry here, or the console cannot use it.
pub const SCHEMAS: [&str; 2] = ["public", "bronze_meta"];

/// The fixed sentence the start path exits with when the console's
/// database role may change the audit trail.
pub const RUNTIME_ROLE_REFUSAL: &str = "The console's database role must not be a superuser or own the tables; see \
     docs/OPERATIONS.md, 'Database roles'";

/// A table the console role may append to, but not change.
#[derive(Debug, Clone, Copy)]
pub struct AppendOnlyTable {
    /// The table, schema-qualified: a `REVOKE` names one exact relation.
    pub table: &'static str,
    /// The columns an `UPDATE` may change. Empty means the table refuses
    /// every change.
    ///
    /// `audit_event` admits none. The `ON DELETE SET NULL` of its
    /// `run_id` and `approval_id` foreign keys runs as the owner of the
    /// referencing table, and Postgres does not check the deleting role's
    /// privileges on that action, so the console role needs no `UPDATE`
    /// to keep the audit row when its run or approval is deleted. A grant
    /// would let the console clear those links by a plain `UPDATE`.
    pub allowed_update_columns: &'static [&'static str],
}

/// The tables the console role may only append to.
///
/// Every table with a refusing trigger in `rust/migrations/` is in this
/// list; the test in `tests/db_roles.rs` fails when a migration adds one
/// that is missing here.
pub const APPEND_ONLY_TABLES: &[AppendOnlyTable] = &[
    AppendOnlyTable {
        table: "public.audit_event",
        allowed_update_columns: &[],
    },
    AppendOnlyTable {
        table: "public.usage_rate",
        allowed_update_columns: &[],
    },
];

/// The `ALTER DEFAULT PRIVILEGES` statements, one per object kind.
///
/// The schema and the role are passed to Postgres' own `format()` as bound
/// values and quoted there (`%I`): a schema name and a role name are
/// identifiers, and this process never builds one by string concatenation.
///
/// No `FOR ROLE`: the statement then covers the objects the CURRENT role
/// creates, and the setup always runs with the owner's role — the role the
/// migrations run as, and the only role that creates tables in these
/// schemas. Naming the role instead would ask the database for a
/// privilege (membership of that role) the setup has no reason to need.
const DEFAULT_PRIVILEGE_TEMPLATES: [&str; 3] = [
    "ALTER DEFAULT PRIVILEGES IN SCHEMA %I \
     GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I",
    "ALTER DEFAULT PRIVILEGES IN SCHEMA %I GRANT USAGE, SELECT ON SEQUENCES TO %I",
    "ALTER DEFAULT PRIVILEGES IN SCHEMA %I GRANT EXECUTE ON FUNCTIONS TO %I",
];

/// Why the role setup failed.
///
/// The upstream error's text never reaches a message (`AGENTS.md`
/// principle 4): the log gets it under a reference id, the operator reads
/// one of the two fixed sentences below.
#[derive(Debug)]
pub enum RoleSetupError {
    /// The database refused one of the setup statements.
    ///
    /// The upstream error is the source of this one; it is what the log
    /// gets, and what the caller may read through [`Self::upstream`].
    Refused(sqlx::Error),
    /// The database refused the statement that carried the password.
    ///
    /// Only the SQLSTATE is kept: the server's own message can echo the
    /// statement it refused, and that statement holds the password. The
    /// password therefore never reaches an error message, a log line, or
    /// a `Debug` print of this value.
    PasswordRefused {
        /// The SQLSTATE the server answered with, when it sent one.
        sqlstate: Option<String>,
    },
}

impl RoleSetupError {
    /// The upstream error, when the refusal was not the password one.
    ///
    /// The caller logs this; it never puts it in a message.
    #[must_use]
    pub fn upstream(&self) -> Option<&sqlx::Error> {
        match self {
            Self::Refused(err) => Some(err),
            Self::PasswordRefused { .. } => None,
        }
    }

    /// The SQLSTATE of the refusal, when there is one.
    ///
    /// OWNED, not borrowed: the server's SQLSTATE comes back from the
    /// driver as a `Cow`, and the driver does not promise that it lives
    /// as long as the error does — so the code is copied here rather than
    /// borrowed out of a temporary.
    #[must_use]
    pub fn sqlstate(&self) -> Option<String> {
        match self {
            Self::Refused(err) => sqlstate_of(err),
            Self::PasswordRefused { sqlstate } => sqlstate.clone(),
        }
    }
}

impl fmt::Display for RoleSetupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(_) => f.write_str("the console's database role could not be set up"),
            Self::PasswordRefused { .. } => {
                f.write_str("the console's database role's password could not be set")
            }
        }
    }
}

impl std::error::Error for RoleSetupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Refused(err) => Some(err),
            Self::PasswordRefused { .. } => None,
        }
    }
}

/// The SQLSTATE the server answered with, when it sent one.
fn sqlstate_of(err: &sqlx::Error) -> Option<String> {
    let sqlx::Error::Database(db) = err else {
        return None;
    };
    db.code().map(std::borrow::Cow::into_owned)
}

/// Set the console's database role up (`SEC-78` decision 2).
///
/// Idempotent: every statement can run again, so the `migrate` command
/// runs this on every deploy, and a rotated password applies on the next
/// one.
///
/// The order is the security posture:
///
/// 1. create the role if it does not exist (race-safe: the role is
///    cluster-wide, so two runs can try at once and the loser of the race
///    catches the duplicate);
/// 2. set its attributes and its password;
/// 3. grant `CONNECT` on this database;
/// 4. grant `USAGE` on every schema in [`SCHEMAS`];
/// 5. grant the table, sequence and function privileges in those schemas;
/// 6. grant the same to the tables a LATER migration creates
///    (`ALTER DEFAULT PRIVILEGES`);
/// 7. revoke the write privileges on every table in
///    [`APPEND_ONLY_TABLES`] and grant back only what the table's trigger
///    admits;
/// 8. grant `pg_monitor`, the built-in role that carries the read-only
///    statistics privileges.
///
/// # Errors
///
/// Returns [`RoleSetupError`] if the database refused one of the
/// statements. The statement that carries the password reports
/// [`RoleSetupError::PasswordRefused`], which keeps its SQLSTATE and
/// nothing else.
pub async fn setup_app_role(pool: &PgPool, password: &SecretValue) -> Result<(), RoleSetupError> {
    create_role_if_missing(pool).await?;
    set_role_attributes_and_password(pool, password).await?;
    grant_connect_on_database(pool).await?;
    for schema in SCHEMAS {
        grant_on_schema(pool, schema).await?;
    }
    keep_migration_ledger_read_only(pool).await?;
    for schema in SCHEMAS {
        grant_default_privileges(pool, schema).await?;
    }
    for table in APPEND_ONLY_TABLES {
        restrict_append_only_table(pool, table).await?;
    }
    grant_pg_monitor(pool).await?;
    Ok(())
}

/// Create the console's role if it does not exist yet.
///
/// Race-safe: the role is cluster-wide, so two `migrate` runs, or two
/// tests in one cluster, can try to create it at the same time. The
/// duplicate is caught by its SQLSTATE (`42710`, `duplicate_object`), so
/// the loser of the race carries on with the role the winner created.
async fn create_role_if_missing(pool: &PgPool) -> Result<(), RoleSetupError> {
    let statement = format!(
        "CREATE ROLE {APP_ROLE_NAME} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOREPLICATION NOBYPASSRLS"
    );
    match sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(pool)
        .await
    {
        Ok(_) => Ok(()),
        Err(err) if sqlstate_of(&err).as_deref() == Some("42710") => Ok(()),
        Err(err) => Err(RoleSetupError::Refused(err)),
    }
}

/// Set the role's attributes and its password.
///
/// The password is bound as a query parameter and never concatenated
/// into SQL text by this process; the server then quotes it (`%L`) into
/// the statement it builds, so the password does end up in the text of
/// exactly ONE statement this process executes.
///
/// That is why this one statement's refusal is reported by SQLSTATE
/// alone and nothing else about the refusal is reported (see
/// [`RoleSetupError::PasswordRefused`]): the server's message for a
/// refused statement can quote the statement, and with it the password.
async fn set_role_attributes_and_password(
    pool: &PgPool,
    password: &SecretValue,
) -> Result<(), RoleSetupError> {
    let statement: String = sqlx::query_scalar(
        "SELECT format('ALTER ROLE %I WITH LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE \
         NOREPLICATION NOBYPASSRLS PASSWORD %L', $1, $2)",
    )
    .bind(APP_ROLE_NAME)
    .bind(password.expose_secret())
    .fetch_one(pool)
    .await
    .map_err(RoleSetupError::Refused)?;
    // The statement carries the plain password as a literal. The server
    // logs a statement's text under the default `log_min_error_statement`
    // (a failing one) and under an operator's `log_statement`, so both are
    // switched off for this one transaction. `SET LOCAL` ends with it, so
    // no other statement of this session is affected. The migrate role is
    // a superuser, which these settings require.
    let mut transaction = pool.begin().await.map_err(RoleSetupError::Refused)?;
    for guard in PASSWORD_LOG_GUARD {
        sqlx::query(guard)
            .execute(&mut *transaction)
            .await
            .map_err(RoleSetupError::Refused)?;
    }
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(&mut *transaction)
        .await
        .map_err(|err| RoleSetupError::PasswordRefused {
            sqlstate: sqlstate_of(&err),
        })?;
    transaction.commit().await.map_err(RoleSetupError::Refused)?;
    Ok(())
}

/// The `SET LOCAL` statements that keep a password statement out of the
/// server log. Constants, so the test in `tests/db_roles.rs` runs the same
/// statements the setup runs.
pub const PASSWORD_LOG_GUARD: [&str; 2] = [
    "SET LOCAL log_statement = 'none'",
    "SET LOCAL log_min_error_statement = 'panic'",
];

/// Grant the role `CONNECT` on the database the console uses.
///
/// The database name is an identifier, so the server quotes it (`%I`); the
/// role name is a constant of this module.
async fn grant_connect_on_database(pool: &PgPool) -> Result<(), RoleSetupError> {
    let statement = format!(
        "DO $$ BEGIN EXECUTE format('GRANT CONNECT ON DATABASE %I TO {APP_ROLE_NAME}', \
         current_database()); END $$;"
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(pool)
        .await
        .map_err(RoleSetupError::Refused)?;
    Ok(())
}

/// Grant the role what it needs in one schema.
///
/// The schema is one of [`SCHEMAS`], a constant list of literals.
async fn grant_on_schema(pool: &PgPool, schema: &str) -> Result<(), RoleSetupError> {
    let statements = [
        format!("GRANT USAGE ON SCHEMA {schema} TO {APP_ROLE_NAME}"),
        format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA {schema} TO \
             {APP_ROLE_NAME}"
        ),
        format!("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA {schema} TO {APP_ROLE_NAME}"),
        format!("GRANT EXECUTE ON ALL FUNCTIONS IN SCHEMA {schema} TO {APP_ROLE_NAME}"),
    ];
    for statement in statements {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
            .map_err(RoleSetupError::Refused)?;
    }
    Ok(())
}

/// Take the write privileges on the migration ledger back.
///
/// The `ALL TABLES` grant above covers `_sqlx_migrations` too. The boot
/// check reads that ledger to decide the schema is current, so the role
/// must not write to it: an INSERT would make a stale schema look
/// current, a DELETE would make a current one look stale. SELECT stays,
/// so the check can still read it.
async fn keep_migration_ledger_read_only(pool: &PgPool) -> Result<(), RoleSetupError> {
    let statement = format!(
        "REVOKE INSERT, UPDATE, DELETE, TRUNCATE ON TABLE _sqlx_migrations FROM {APP_ROLE_NAME}"
    );
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(pool)
        .await
        .map_err(RoleSetupError::Refused)?;
    Ok(())
}

/// Grant the role the same privileges on the tables a LATER migration
/// creates.
///
/// `ALTER DEFAULT PRIVILEGES` covers every table the current role (the
/// owner) creates from now on.
async fn grant_default_privileges(pool: &PgPool, schema: &str) -> Result<(), RoleSetupError> {
    for template in DEFAULT_PRIVILEGE_TEMPLATES {
        let statement: String = sqlx::query_scalar("SELECT format($1, $2, $3)")
            .bind(template)
            .bind(schema)
            .bind(APP_ROLE_NAME)
            .fetch_one(pool)
            .await
            .map_err(RoleSetupError::Refused)?;
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
            .map_err(RoleSetupError::Refused)?;
    }
    Ok(())
}

/// Take the write privileges back on one append-only table.
///
/// The blanket grants gave the role `UPDATE` and `DELETE` on every table;
/// this takes them back and grants back only what the table's trigger
/// admits, so the grant and the trigger agree exactly.
async fn restrict_append_only_table(
    pool: &PgPool,
    table: &AppendOnlyTable,
) -> Result<(), RoleSetupError> {
    let statements = [
        format!(
            "REVOKE UPDATE, DELETE, TRUNCATE, TRIGGER, REFERENCES ON TABLE {} FROM \
             {APP_ROLE_NAME}",
            table.table
        ),
        format!(
            "GRANT SELECT, INSERT ON TABLE {} TO {APP_ROLE_NAME}",
            table.table
        ),
    ];
    for statement in statements {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
            .map_err(RoleSetupError::Refused)?;
    }
    if !table.allowed_update_columns.is_empty() {
        let columns = table.allowed_update_columns.join(", ");
        let statement = format!(
            "GRANT UPDATE ({columns}) ON TABLE {} TO {APP_ROLE_NAME}",
            table.table
        );
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(pool)
            .await
            .map_err(RoleSetupError::Refused)?;
    }
    Ok(())
}

/// Grant the built-in `pg_monitor` role.
///
/// It carries the read-only statistics privileges (`pg_read_all_stats`,
/// `pg_read_all_settings`, `pg_stat_scan_tables`) that the jobs reading
/// the console database need, and nothing that could switch off the
/// append-only triggers.
async fn grant_pg_monitor(pool: &PgPool) -> Result<(), RoleSetupError> {
    let statement = format!("GRANT pg_monitor TO {APP_ROLE_NAME}");
    sqlx::query(sqlx::AssertSqlSafe(statement))
        .execute(pool)
        .await
        .map_err(RoleSetupError::Refused)?;
    Ok(())
}

/// Why the start path refuses to serve.
#[derive(Debug)]
pub enum RuntimeRoleRefusal {
    /// The role the API connects as is a superuser, or owns a table in one
    /// of the console's schemas. Either one can switch off the append-only
    /// triggers.
    SuperuserOrOwner,
    /// The check itself could not be read from the database. The raw
    /// `sqlx` error is the source of this one; it never reaches a message.
    Unreadable(sqlx::Error),
}

impl RuntimeRoleRefusal {
    /// The upstream error, when the refusal was not the role's own doing.
    ///
    /// The caller logs this; it never puts it in a message.
    #[must_use]
    pub fn upstream(&self) -> Option<&sqlx::Error> {
        match self {
            Self::SuperuserOrOwner => None,
            Self::Unreadable(err) => Some(err),
        }
    }
}

impl fmt::Display for RuntimeRoleRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SuperuserOrOwner => f.write_str(RUNTIME_ROLE_REFUSAL),
            Self::Unreadable(_) => {
                f.write_str("the console's database role could not be checked; refusing to start")
            }
        }
    }
}

impl std::error::Error for RuntimeRoleRefusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SuperuserOrOwner => None,
            Self::Unreadable(err) => Some(err),
        }
    }
}

/// Whether the role the API connects as may serve (`SEC-78` decision 5).
///
/// A superuser can switch off any trigger, and the owner of a table can
/// switch off that table's triggers — either one gets past the
/// append-only rule of `audit_event`. So the API refuses to start as
/// either.
///
/// Read-only: it reads the catalog, and nothing else.
///
/// # Errors
///
/// Returns [`RuntimeRoleRefusal::Unreadable`] if the check cannot be read
/// from the database.
pub async fn check_runtime_role(pool: &PgPool) -> Result<(), RuntimeRoleRefusal> {
    let schemas: Vec<String> = SCHEMAS.iter().map(|schema| String::from(*schema)).collect();
    let is_superuser_or_owner: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = current_user AND rolsuper) \
             OR EXISTS (SELECT 1 FROM pg_tables \
                        WHERE schemaname::text = ANY($1) AND tableowner = current_user)",
    )
    .bind(schemas)
    .fetch_one(pool)
    .await
    .map_err(RuntimeRoleRefusal::Unreadable)?;
    if is_superuser_or_owner {
        Err(RuntimeRoleRefusal::SuperuserOrOwner)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The password must not be in the error the caller sees: the
    /// statement that carries it is refused by SQLSTATE alone.
    #[test]
    fn a_refused_password_carries_only_its_sqlstate() {
        let err = RoleSetupError::PasswordRefused {
            sqlstate: Some("42501".to_owned()),
        };
        assert_eq!(err.sqlstate(), Some("42501".to_owned()));
        assert!(err.upstream().is_none());
        assert!(
            !err.to_string().contains("42501"),
            "the message is a fixed sentence, not the code: {err}"
        );
        assert_eq!(
            err.to_string(),
            "the console's database role's password could not be set"
        );
    }

    /// A refused statement that did NOT carry the password keeps the
    /// upstream error for the log — and the message stays the fixed
    /// sentence, never the upstream's own text.
    #[test]
    fn a_refused_statement_keeps_the_upstream_error_for_the_log() {
        let err = RoleSetupError::Refused(sqlx::Error::Protocol("planted-marker-text".to_owned()));
        assert!(err.upstream().is_some());
        assert!(!err.to_string().contains("planted-marker-text"));
        assert_eq!(
            err.to_string(),
            "the console's database role could not be set up"
        );
    }

    /// The refusal of the start path is the fixed sentence the plan names,
    /// in every case.
    #[test]
    fn the_start_path_refusal_is_the_fixed_sentence() {
        assert_eq!(
            RuntimeRoleRefusal::SuperuserOrOwner.to_string(),
            "The console's database role must not be a superuser or own the tables; see \
             docs/OPERATIONS.md, 'Database roles'"
        );
        assert_eq!(
            RuntimeRoleRefusal::SuperuserOrOwner.to_string(),
            RUNTIME_ROLE_REFUSAL
        );
        assert!(RuntimeRoleRefusal::SuperuserOrOwner.upstream().is_none());
    }
}
