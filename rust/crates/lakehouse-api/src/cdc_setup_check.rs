//! What a change-capture source must have switched on, as a list of checks
//! (`SRC-4` slice 4, decision S4-f, feature page decision 9). Pure: the
//! functions here turn the facts a probe read into [`SetupCheck`]s and read
//! nothing themselves; [`crate::cdc_setup_probe`] reads the facts.
//!
//! # Every word is ours
//!
//! A check's `message` and `command` come from the constants and templates
//! below, never from the server (principle 4). A template takes only values
//! this build already holds and validated: the connector's own slot and
//! publication names, its login, and the tables its spec selects. They are
//! displayed, never executed, and quoted for the engine they are shown for.
//!
//! # Fixed statements
//!
//! The statements that read the facts are constants ([`PG_STATEMENTS`] and the
//! `MYSQL_*` / `MSSQL_*` ones in the probe). No value is put in a statement:
//! the publication name and the selected tables are compared here, in Rust,
//! against everything the fixed statement returned.
//!
//! # `unchecked` is not a pass
//!
//! A fact that could not be read (a statement the login may not run, a
//! connection that ended) gives `unchecked` with a fixed sentence, never `ok`.

use lakehouse_store::connector_discovery::{
    ChangeStreamFact, MongoSetupFacts, OracleLogMode, OraclePrivilege, OracleSetupFacts,
    OracleSupplementalMin, OracleTableFact, ReplicaSetFact,
};
use lakehouse_store::connectors::{SetupCheck, SetupStatus};
use lakehouse_store::ingest_spec::CdcMongoDial;

/// Rows of text values, `None` for SQL `NULL`.
pub type Rows = Vec<Vec<Option<String>>>;

/// The shortest binlog retention that is not a risk: 7 days, the broker's own
/// retention (`SRC-4` decision S2-i), the window in which a stopped connector
/// can still catch up. It is the planner's number (S4-f).
pub const MIN_BINLOG_RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;

/// Shown when a setting could not be read.
const UNREAD: &str = "Could not be read with this login, usually because it lacks a permission. \
                      Check it on the server.";

fn ok(setting: &str, message: &str) -> SetupCheck {
    SetupCheck {
        setting: setting.to_owned(),
        status: SetupStatus::Ok,
        message: message.to_owned(),
        command: None,
    }
}

fn missing(setting: &str, message: &str, command: String) -> SetupCheck {
    SetupCheck {
        setting: setting.to_owned(),
        status: SetupStatus::Missing,
        message: message.to_owned(),
        command: Some(command),
    }
}

fn unchecked(setting: &str, message: &str) -> SetupCheck {
    SetupCheck {
        setting: setting.to_owned(),
        status: SetupStatus::Unchecked,
        message: message.to_owned(),
        command: None,
    }
}

/// The first cell of the first row, if there is one.
fn first_cell(rows: Option<&Rows>) -> Option<&str> {
    rows?.first()?.first()?.as_deref()
}

// ── PostgreSQL ─────────────────────────────────────────────────────────────

/// The fixed statements of the `PostgreSQL` checks, in the order of [`PgFacts`].
pub const PG_STATEMENTS: [&str; 4] = [
    "SHOW wal_level",
    "SELECT rolreplication OR rolsuper FROM pg_roles WHERE rolname = current_user",
    "SELECT pubname FROM pg_publication",
    "SELECT pubname, schemaname, tablename FROM pg_publication_tables",
];

/// What the [`PG_STATEMENTS`] returned; `None` for one that could not be read.
#[derive(Debug, Default)]
pub struct PgFacts {
    /// `SHOW wal_level`.
    pub wal_level: Option<Rows>,
    /// Whether the login may replicate (`t` or `f`).
    pub replication: Option<Rows>,
    /// Every publication's name.
    pub publications: Option<Rows>,
    /// Every publication's tables.
    pub published: Option<Rows>,
}

/// `"name"` with embedded quotes doubled.
fn pg_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// `"schema"."table"` for `schema.table`; a name with no dot is quoted whole.
fn pg_table(qualified: &str) -> String {
    match qualified.split_once('.') {
        Some((schema, table)) => format!("{}.{}", pg_ident(schema), pg_ident(table)),
        None => pg_ident(qualified),
    }
}

fn pg_tables(tables: &[&str]) -> String {
    tables
        .iter()
        .map(|t| pg_table(t))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The `PostgreSQL` checks: `wal_level`, the login's replication right, the
/// publication, and the publication's tables.
#[must_use]
pub fn postgres_checks(
    facts: &PgFacts,
    user: &str,
    publication: &str,
    tables: &[String],
) -> Vec<SetupCheck> {
    let mut checks = vec![
        pg_wal_level(facts),
        pg_replication(facts, user),
        pg_publication(facts, publication, tables),
    ];
    if let Some(published) = pg_published(facts, publication, tables) {
        checks.push(published);
    }
    checks
}

fn pg_wal_level(facts: &PgFacts) -> SetupCheck {
    const SETTING: &str = "wal_level";
    match first_cell(facts.wal_level.as_ref()) {
        None => unchecked(SETTING, UNREAD),
        Some("logical") => ok(SETTING, "wal_level is logical."),
        Some(_) => missing(
            SETTING,
            "wal_level must be logical for change capture. Changing it needs a restart of PostgreSQL.",
            "ALTER SYSTEM SET wal_level = 'logical';".to_owned(),
        ),
    }
}

fn pg_replication(facts: &PgFacts, user: &str) -> SetupCheck {
    const SETTING: &str = "replication role";
    match first_cell(facts.replication.as_ref()) {
        None => unchecked(SETTING, UNREAD),
        Some("t") => ok(SETTING, "The login may replicate."),
        Some(_) => missing(
            SETTING,
            "The login needs the REPLICATION attribute (or to be a superuser).",
            format!("ALTER ROLE {} REPLICATION;", pg_ident(user)),
        ),
    }
}

fn pg_publication(facts: &PgFacts, publication: &str, tables: &[String]) -> SetupCheck {
    const SETTING: &str = "publication";
    let Some(rows) = facts.publications.as_ref() else {
        return unchecked(SETTING, UNREAD);
    };
    let exists = rows
        .iter()
        .any(|row| row.first().and_then(|c| c.as_deref()) == Some(publication));
    if exists {
        return ok(SETTING, "The publication exists.");
    }
    let names: Vec<&str> = tables.iter().map(String::as_str).collect();
    missing(
        SETTING,
        "The publication named in the connector does not exist.",
        if names.is_empty() {
            format!("CREATE PUBLICATION {};", pg_ident(publication))
        } else {
            format!(
                "CREATE PUBLICATION {} FOR TABLE {};",
                pg_ident(publication),
                pg_tables(&names)
            )
        },
    )
}

/// The publication-tables check; `None` when the publication does not exist,
/// because creating it (the command of the publication check) names every
/// selected table.
fn pg_published(facts: &PgFacts, publication: &str, tables: &[String]) -> Option<SetupCheck> {
    const SETTING: &str = "publication tables";
    let exists = facts.publications.as_ref().map(|rows| {
        rows.iter()
            .any(|row| row.first().and_then(|c| c.as_deref()) == Some(publication))
    });
    if exists == Some(false) {
        return None;
    }
    let Some(rows) = facts.published.as_ref() else {
        return Some(unchecked(SETTING, UNREAD));
    };
    let held = |table: &str| {
        rows.iter().any(|row| {
            let cell = |i: usize| row.get(i).and_then(|c| c.as_deref());
            cell(0) == Some(publication)
                && matches!((cell(1), cell(2)), (Some(s), Some(t)) if format!("{s}.{t}") == table)
        })
    };
    let absent: Vec<&str> = tables
        .iter()
        .map(String::as_str)
        .filter(|t| !held(t))
        .collect();
    if absent.is_empty() {
        return Some(ok(SETTING, "The publication holds every selected table."));
    }
    Some(missing(
        SETTING,
        "The publication does not hold every selected table.",
        format!(
            "ALTER PUBLICATION {} ADD TABLE {};",
            pg_ident(publication),
            pg_tables(&absent)
        ),
    ))
}

// ── MySQL ──────────────────────────────────────────────────────────────────

/// What the `MySQL` statements returned; `None` for one that could not be read.
#[derive(Debug, Default)]
pub struct MysqlFacts {
    /// `@@log_bin` as text (`1` or `0`).
    pub log_bin: Option<String>,
    /// `@@binlog_format`.
    pub binlog_format: Option<String>,
    /// `@@binlog_row_image`.
    pub binlog_row_image: Option<String>,
    /// The lines of `SHOW GRANTS FOR CURRENT_USER()`.
    pub grants: Option<Vec<String>>,
    /// `@@binlog_expire_logs_seconds` as text.
    pub expire_seconds: Option<String>,
    /// `@@expire_logs_days` as text; `0` where the variable no longer exists (8.4).
    pub expire_days: Option<String>,
    /// `CURRENT_USER()`, `user@host`: the account a `GRANT` must name.
    pub current_user: Option<String>,
}

fn mysql_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

/// The `MySQL` checks: the binary log, its format and row image, the login's
/// replication privileges and the binlog retention.
#[must_use]
pub fn mysql_checks(facts: &MysqlFacts, user: &str) -> Vec<SetupCheck> {
    // Slice 4 review SHOULD-FIX 8: the account is the one the server says, not the dial's
    // user at an invented host.
    let account = match facts
        .current_user
        .as_deref()
        .and_then(|u| u.rsplit_once('@'))
    {
        Some((name, host)) => format!("{}@{}", mysql_string(name), mysql_string(host)),
        None => format!("{}@'%'", mysql_string(user)),
    };
    vec![
        mysql_log_bin(facts),
        mysql_setting(
            "binlog_format",
            facts.binlog_format.as_deref(),
            "ROW",
            "binlog_format is ROW.",
            "binlog_format must be ROW for change capture.",
        ),
        mysql_setting(
            "binlog_row_image",
            facts.binlog_row_image.as_deref(),
            "FULL",
            "binlog_row_image is FULL.",
            "binlog_row_image must be FULL for change capture.",
        ),
        mysql_grants(facts, &account),
        mysql_retention(facts),
    ]
}

fn mysql_log_bin(facts: &MysqlFacts) -> SetupCheck {
    const SETTING: &str = "binary log";
    match facts.log_bin.as_deref() {
        None => unchecked(SETTING, UNREAD),
        Some(v) if v == "1" || v.eq_ignore_ascii_case("ON") => ok(SETTING, "The binary log is on."),
        Some(_) => missing(
            SETTING,
            "The binary log must be on for change capture. Changing it needs a restart of MySQL.",
            "log_bin = mysql-bin   # in the [mysqld] section of the server configuration"
                .to_owned(),
        ),
    }
}

fn mysql_setting(
    setting: &str,
    value: Option<&str>,
    wanted: &str,
    ok_message: &str,
    missing_message: &str,
) -> SetupCheck {
    match value {
        None => unchecked(setting, UNREAD),
        Some(v) if v.eq_ignore_ascii_case(wanted) => ok(setting, ok_message),
        Some(_) => missing(
            setting,
            missing_message,
            format!("SET PERSIST {setting} = '{wanted}';"),
        ),
    }
}

/// Whether the grants hold the two replication privileges on every database
/// (`ON *.*`), directly or through `ALL PRIVILEGES`.
fn has_replication_grants(grants: &[String]) -> bool {
    // Only the privilege list between `GRANT ` and ` ON *.*` counts (a role name or a
    // table name that contains a privilege's words must not).
    let mut held: Vec<String> = Vec::new();
    for grant in grants {
        let upper = grant.to_ascii_uppercase();
        let Some(rest) = upper.strip_prefix("GRANT ") else {
            continue;
        };
        let Some((privileges, scope)) = rest.split_once(" ON ") else {
            continue;
        };
        if scope.starts_with("*.* ") {
            held.extend(privileges.split(',').map(|p| p.trim().to_owned()));
        }
    }
    held.iter().any(|p| p == "ALL PRIVILEGES")
        || (held.iter().any(|p| p == "REPLICATION SLAVE")
            && held.iter().any(|p| p == "REPLICATION CLIENT"))
}

fn mysql_grants(facts: &MysqlFacts, account: &str) -> SetupCheck {
    const SETTING: &str = "replication privileges";
    match facts.grants.as_deref() {
        None => unchecked(SETTING, UNREAD),
        Some(grants) if has_replication_grants(grants) => ok(
            SETTING,
            "The login holds REPLICATION SLAVE and REPLICATION CLIENT.",
        ),
        Some(_) => missing(
            SETTING,
            "The login needs REPLICATION SLAVE and REPLICATION CLIENT.",
            format!("GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO {account};"),
        ),
    }
}

fn mysql_retention(facts: &MysqlFacts) -> SetupCheck {
    const SETTING: &str = "binlog retention";
    let seconds = facts
        .expire_seconds
        .as_deref()
        .and_then(|v| v.trim().parse::<i64>().ok());
    let days = facts
        .expire_days
        .as_deref()
        .and_then(|v| v.trim().parse::<i64>().ok());
    let (Some(seconds), Some(days)) = (seconds, days) else {
        return unchecked(SETTING, UNREAD);
    };
    // Slice 4 review SHOULD-FIX 6: 0 seconds is "never expire" only when the legacy
    // `expire_logs_days` is 0 too; a non-zero `binlog_expire_logs_seconds` takes precedence.
    let effective = if seconds != 0 {
        seconds
    } else {
        days.saturating_mul(24 * 60 * 60)
    };
    match effective {
        0 => ok(SETTING, "Binary logs never expire."),
        s if s >= MIN_BINLOG_RETENTION_SECONDS => {
            ok(SETTING, "Binary logs are kept for at least 7 days.")
        }
        _ => missing(
            SETTING,
            "Binary logs expire in under 7 days, so a connector stopped for longer cannot catch up.",
            format!("SET PERSIST binlog_expire_logs_seconds = {MIN_BINLOG_RETENTION_SECONDS};"),
        ),
    }
}

// ── SQL Server ─────────────────────────────────────────────────────────────

/// Whether `SQL Server Agent` runs, as far as it could be read.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// The statement was refused (it needs `VIEW SERVER STATE`) or failed.
    #[default]
    Unread,
    /// The service is running.
    Running,
    /// The service is listed and is not running.
    NotRunning,
    /// The statement ran and listed no Agent (an edition or platform without one):
    /// not stopped, not known (Slice 4 review SHOULD-FIX 7).
    NotListed,
}

/// What the `SQL Server` statements returned; `None` for one that could not be
/// read.
#[derive(Debug, Default)]
pub struct MssqlFacts {
    /// `is_cdc_enabled` of the connected database.
    pub cdc_enabled: Option<bool>,
    /// `(schema, table, is_tracked_by_cdc)` of every table of the database.
    pub tables: Option<Vec<(String, String, bool)>>,
    /// The Agent's state.
    pub agent: AgentState,
}

/// `N'value'` with embedded quotes doubled.
fn mssql_string(value: &str) -> String {
    format!("N'{}'", value.replace('\'', "''"))
}

/// The `SQL Server` checks: `CDC` on the database, `CDC` on each selected
/// table, and the Agent.
#[must_use]
pub fn mssql_checks(facts: &MssqlFacts, tables: &[String]) -> Vec<SetupCheck> {
    let mut checks = vec![mssql_database(facts)];
    checks.extend(tables.iter().map(|table| mssql_table(facts, table)));
    checks.push(mssql_agent(facts));
    checks
}

fn mssql_database(facts: &MssqlFacts) -> SetupCheck {
    const SETTING: &str = "CDC on the database";
    match facts.cdc_enabled {
        None => unchecked(SETTING, UNREAD),
        Some(true) => ok(SETTING, "Change data capture is enabled on the database."),
        Some(false) => missing(
            SETTING,
            "Change data capture must be enabled on the database.",
            "EXEC sys.sp_cdc_enable_db;".to_owned(),
        ),
    }
}

fn mssql_table(facts: &MssqlFacts, qualified: &str) -> SetupCheck {
    let setting = format!("CDC on table {qualified}");
    let Some(listed) = facts.tables.as_ref() else {
        return unchecked(&setting, UNREAD);
    };
    let (schema, table) = qualified.split_once('.').unwrap_or(("dbo", qualified));
    let tracked = listed.iter().any(|(s, t, tracked)| {
        *tracked && s.eq_ignore_ascii_case(schema) && t.eq_ignore_ascii_case(table)
    });
    if tracked {
        return ok(&setting, "Change data capture is enabled on the table.");
    }
    missing(
        &setting,
        "Change data capture must be enabled on the table.",
        format!(
            "EXEC sys.sp_cdc_enable_table @source_schema = {}, @source_name = {}, @role_name = NULL;",
            mssql_string(schema),
            mssql_string(table)
        ),
    )
}

fn mssql_agent(facts: &MssqlFacts) -> SetupCheck {
    const SETTING: &str = "SQL Server Agent";
    match facts.agent {
        AgentState::Unread => unchecked(
            SETTING,
            "Reading it needs VIEW SERVER STATE, which the documented login does not have. \
             Check on the server that SQL Server Agent runs: the capture job runs under it.",
        ),
        AgentState::NotListed => unchecked(
            SETTING,
            "This server lists no SQL Server Agent (some editions and platforms do not). \
             Check on the server that it runs: the capture job runs under it.",
        ),
        AgentState::Running => ok(SETTING, "SQL Server Agent is running."),
        AgentState::NotRunning => SetupCheck {
            setting: SETTING.to_owned(),
            status: SetupStatus::Missing,
            message: "SQL Server Agent is not running. The capture job runs under it, so no change is captured."
                .to_owned(),
            command: None,
        },
    }
}

// ── MongoDB ────────────────────────────────────────────────────────────────

/// `value` as a JSON string literal, which `mongosh` reads as the same string: quotes,
/// backslashes and control characters are escaped by `serde_json`, never by hand. The user is
/// any string the dial accepted (it only has to be non-empty).
fn json_string(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

fn missing_without_command(setting: &str, message: &str) -> SetupCheck {
    SetupCheck {
        setting: setting.to_owned(),
        status: SetupStatus::Missing,
        message: message.to_owned(),
        command: None,
    }
}

/// The `MongoDB` checks (`SRC-4` Amendment 8, S8-d): the replica set's name, that the one host
/// the connector names is the primary, and that the login may read change streams on its
/// database. Pure: `facts` are what the orchestrator's connection test posted (closed words,
/// no server text), and a fact it did not read, or no facts at all, is `unchecked`, never `ok`.
/// Only the change-stream check carries a command, built from the dial's database and user.
#[must_use]
pub fn mongo_checks(facts: Option<&MongoSetupFacts>, dial: &CdcMongoDial) -> Vec<SetupCheck> {
    let facts = facts.copied().unwrap_or_default();
    vec![
        mongo_replica_set(facts.replica_set),
        mongo_primary(facts.primary),
        mongo_change_stream(facts.change_stream, dial),
    ]
}

fn mongo_replica_set(fact: Option<ReplicaSetFact>) -> SetupCheck {
    const SETTING: &str = "replicaSet";
    match fact {
        None => unchecked(SETTING, UNREAD),
        Some(ReplicaSetFact::Matches) => ok(
            SETTING,
            "The server's replica set name is the one named here.",
        ),
        Some(ReplicaSetFact::Differs) => missing_without_command(
            SETTING,
            "The server's replica set has another name. Use the name in `rs.status().set`.",
        ),
        Some(ReplicaSetFact::None) => missing_without_command(
            SETTING,
            "The server is not a replica set member; change streams need a replica set.",
        ),
    }
}

fn mongo_primary(fact: Option<bool>) -> SetupCheck {
    const SETTING: &str = "primary";
    match fact {
        None => unchecked(SETTING, UNREAD),
        Some(true) => ok(SETTING, "This host is the primary."),
        // Slice 5 review BLOCKER 1: the connector reads through the one host it names.
        Some(false) => missing_without_command(
            SETTING,
            "This host is not the primary. The connector reads change streams through the one host it names, so name the primary.",
        ),
    }
}

fn mongo_change_stream(fact: Option<ChangeStreamFact>, dial: &CdcMongoDial) -> SetupCheck {
    const SETTING: &str = "changeStream";
    match fact {
        None => unchecked(SETTING, UNREAD),
        Some(ChangeStreamFact::Granted) => ok(
            SETTING,
            "The login may read change streams on the database.",
        ),
        Some(ChangeStreamFact::Missing) => {
            let database = json_string(&dial.database);
            missing(
                SETTING,
                "The login may not read change streams on the database. Grant it the read role.",
                format!(
                    "db.getSiblingDB({database}).grantRolesToUser({}, [{{ role: \"read\", db: {database} }}])",
                    json_string(&dial.user)
                ),
            )
        }
    }
}

// ── Oracle ─────────────────────────────────────────────────────────────────

/// What every Oracle check says when the server refused an unencrypted session (`SRC-4`
/// Amendment 11, S11-c): the read is made over one, so nothing could be read.
pub const ORACLE_ENCRYPTION_REQUIRED: &str = "The server requires encryption, which the setup check \
    cannot use: check these settings by hand.";

/// `"name"` with embedded quotes doubled, in Oracle's spelling (upper case): a name typed or
/// created without quotes is stored upper case, and the person pastes this into `SQL*Plus`.
fn oracle_ident(name: &str) -> String {
    format!("\"{}\"", name.to_uppercase().replace('"', "\"\""))
}

/// `"SCHEMA"."TABLE"` for `SCHEMA.TABLE`.
fn oracle_table(qualified: &str) -> String {
    match qualified.split_once('.') {
        Some((schema, table)) => format!("{}.{}", oracle_ident(schema), oracle_ident(table)),
        None => oracle_ident(qualified),
    }
}

/// The Oracle checks (`SRC-4` Amendment 11, S11-b): `ARCHIVELOG`, minimal supplemental logging,
/// all-column logging for each selected table, and the documented `LogMiner` privileges. Pure:
/// `facts` are what the orchestrator's Oracle test posted (closed words, no server text); a fact
/// it did not read, or no facts at all, is `unchecked`, never `ok`, and a server that required
/// encryption makes every check `unchecked` with [`ORACLE_ENCRYPTION_REQUIRED`]. Database-level
/// all-column logging makes every table `ok`. `tables` are the connector's selected tables and
/// `user` the dial's login, shown in the commands only.
#[must_use]
pub fn oracle_checks(
    facts: Option<&OracleSetupFacts>,
    tables: &[String],
    user: &str,
) -> Vec<SetupCheck> {
    let mut settings = vec!["ARCHIVELOG".to_owned(), "Supplemental logging".to_owned()];
    settings.extend(tables.iter().map(|t| format!("All-column logging on {t}")));
    settings.push("LogMiner privileges".to_owned());
    if facts.is_some_and(|f| f.encryption_required == Some(true)) {
        return settings
            .iter()
            .map(|setting| unchecked(setting, ORACLE_ENCRYPTION_REQUIRED))
            .collect();
    }
    let default = OracleSetupFacts::default();
    let facts = facts.unwrap_or(&default);
    let mut checks = vec![oracle_archivelog(facts), oracle_supplemental(facts)];
    checks.extend(tables.iter().map(|table| oracle_table_check(facts, table)));
    checks.push(oracle_privileges(facts, user));
    checks
}

fn oracle_archivelog(facts: &OracleSetupFacts) -> SetupCheck {
    const SETTING: &str = "ARCHIVELOG";
    match facts.log_mode {
        None => unchecked(SETTING, UNREAD),
        Some(OracleLogMode::Archivelog) => ok(SETTING, "The database runs in ARCHIVELOG mode."),
        // No one-line command: it needs a restart, so the message names the steps (S6-g).
        Some(OracleLogMode::Noarchivelog) => missing_without_command(
            SETTING,
            "The database must run in ARCHIVELOG mode. It needs a restart: as SYSDBA run \
             SHUTDOWN IMMEDIATE, STARTUP MOUNT, ALTER DATABASE ARCHIVELOG, ALTER DATABASE OPEN.",
        ),
    }
}

fn oracle_supplemental(facts: &OracleSetupFacts) -> SetupCheck {
    const SETTING: &str = "Supplemental logging";
    match facts.supplemental_min {
        None => unchecked(SETTING, UNREAD),
        Some(OracleSupplementalMin::Yes | OracleSupplementalMin::Implicit) => ok(
            SETTING,
            "Minimal supplemental logging is on for the database.",
        ),
        Some(OracleSupplementalMin::No) => missing(
            SETTING,
            "Minimal supplemental logging must be on for the database.",
            "ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;".to_owned(),
        ),
    }
}

fn oracle_table_check(facts: &OracleSetupFacts, table: &str) -> SetupCheck {
    let setting = format!("All-column logging on {table}");
    if facts.supplemental_all == Some(true) {
        return ok(&setting, "All-column logging is on for the whole database.");
    }
    let Some(found) = facts.tables.as_ref().map(|t| t.get(table)) else {
        return unchecked(&setting, UNREAD);
    };
    match found {
        // The orchestrator was asked about the tables the connector selected when the test
        // started; one chosen since is not in its answer.
        None => unchecked(&setting, UNREAD),
        Some(OracleTableFact::AllColumns) => {
            ok(&setting, "All-column logging is on for the table.")
        }
        Some(OracleTableFact::WithoutAllColumns) => missing(
            &setting,
            "All-column logging must be on for the table. Without it an update arrives with its \
             unchanged columns missing.",
            format!(
                "ALTER TABLE {} ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;",
                oracle_table(table)
            ),
        ),
        Some(OracleTableFact::NotFound) => missing_without_command(
            &setting,
            "No table with this name was found in the database. Check the spelling: Oracle \
             names are upper case. Nothing is captured from a table that does not exist.",
        ),
    }
}

fn oracle_privileges(facts: &OracleSetupFacts, user: &str) -> SetupCheck {
    const SETTING: &str = "LogMiner privileges";
    let Some(absent) = facts.missing_privileges.as_ref() else {
        return unchecked(SETTING, UNREAD);
    };
    let mut required: Vec<OraclePrivilege> = absent
        .iter()
        .copied()
        .filter(|p| {
            !matches!(
                p,
                OraclePrivilege::FlashbackAnyTable | OraclePrivilege::SelectAnyTable
            )
        })
        .collect();
    required.sort();
    required.dedup();
    if !required.is_empty() {
        let names: Vec<&str> = required.iter().map(|p| oracle_privilege_name(*p)).collect();
        return missing(
            SETTING,
            "The login lacks privileges the connector needs.",
            format!(
                "GRANT {} TO {} CONTAINER=ALL;",
                names.join(", "),
                oracle_ident(user)
            ),
        );
    }
    // SELECT ANY TABLE and FLASHBACK ANY TABLE can be granted per table instead, which a
    // session in the container database cannot see: not a failure, not a pass.
    if absent.is_empty() {
        ok(SETTING, "The login holds the documented privileges.")
    } else {
        unchecked(
            SETTING,
            "SELECT ANY TABLE or FLASHBACK ANY TABLE is not granted. That is fine if SELECT and \
             FLASHBACK are granted on each captured table instead: check it by hand.",
        )
    }
}

/// The two messages `Debezium` 3.7.0.Final throws for a source that is not set up, matched in a
/// failed Oracle connector's trace (`SRC-4` Amendment 11, S11-d). Each is cited with its place in
/// the `debezium-connector-oracle` module of the `v3.7.0.Final` tag; a trace that carries neither
/// is not explained, and its text is never shown (principle 4).
///
/// - `OracleConnectorTask.java:316-319` (`validateRedoLogConfiguration`): "... Change the Oracle
///   configuration to use a `LOG_MODE=ARCHIVELOG` and restart the connector."
/// - `logminer/AbstractLogMinerStreamingChangeEventSource.java:1850-1851`
///   (`checkDatabaseAndTableState`): "Supplemental logging not properly configured. Use: ALTER
///   DATABASE ADD SUPPLEMENTAL LOG DATA".
///
/// A table without all-column logging is NOT among them: Debezium only logs a warning for it
/// (same file, lines 1859-1862) and keeps running, so a failure never shows it.
const TRACE_ARCHIVELOG: &str = "LOG_MODE=ARCHIVELOG and restart the connector";
const TRACE_SUPPLEMENTAL: &str = "Supplemental logging not properly configured";

/// The setup checks a failed Oracle connector's `trace` explains: `missing` for each of the two
/// settings above whose message it carries. Empty for any other trace. Pure.
#[must_use]
pub fn oracle_trace_checks(trace: &str) -> Vec<SetupCheck> {
    let mut checks = Vec::new();
    if trace.contains(TRACE_ARCHIVELOG) {
        checks.push(oracle_archivelog(&OracleSetupFacts {
            log_mode: Some(OracleLogMode::Noarchivelog),
            ..OracleSetupFacts::default()
        }));
    }
    if trace.contains(TRACE_SUPPLEMENTAL) {
        checks.push(oracle_supplemental(&OracleSetupFacts {
            supplemental_min: Some(OracleSupplementalMin::No),
            ..OracleSetupFacts::default()
        }));
    }
    checks
}

const fn oracle_privilege_name(privilege: OraclePrivilege) -> &'static str {
    match privilege {
        OraclePrivilege::CreateSession => "CREATE SESSION",
        OraclePrivilege::SetContainer => "SET CONTAINER",
        OraclePrivilege::Logmining => "LOGMINING",
        OraclePrivilege::SelectAnyTransaction => "SELECT ANY TRANSACTION",
        OraclePrivilege::ExecuteCatalogRole => "EXECUTE_CATALOG_ROLE",
        OraclePrivilege::SelectCatalogRole => "SELECT_CATALOG_ROLE",
        OraclePrivilege::CreateTable => "CREATE TABLE",
        OraclePrivilege::LockAnyTable => "LOCK ANY TABLE",
        OraclePrivilege::CreateSequence => "CREATE SEQUENCE",
        OraclePrivilege::FlashbackAnyTable => "FLASHBACK ANY TABLE",
        OraclePrivilege::SelectAnyTable => "SELECT ANY TABLE",
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn rows(values: &[&[&str]]) -> Rows {
        values
            .iter()
            .map(|row| row.iter().map(|c| Some((*c).to_owned())).collect())
            .collect()
    }

    fn statuses(checks: &[SetupCheck]) -> Vec<SetupStatus> {
        checks.iter().map(|c| c.status).collect()
    }

    fn pg_ready() -> PgFacts {
        PgFacts {
            wal_level: Some(rows(&[&["logical"]])),
            replication: Some(rows(&[&["t"]])),
            publications: Some(rows(&[&["other_pub"], &["orders_pub"]])),
            published: Some(rows(&[
                &["orders_pub", "public", "orders"],
                &["orders_pub", "public", "items"],
                &["other_pub", "public", "extra"],
            ])),
        }
    }

    fn tables() -> Vec<String> {
        vec!["public.orders".to_owned(), "public.items".to_owned()]
    }

    #[test]
    fn a_ready_postgres_source_passes_every_check_with_no_command() {
        let checks = postgres_checks(&pg_ready(), "app", "orders_pub", &tables());
        assert_eq!(checks.len(), 4);
        assert!(statuses(&checks).iter().all(|s| *s == SetupStatus::Ok));
        assert!(checks.iter().all(|c| c.command.is_none()));
    }

    #[test]
    fn each_missing_postgres_setting_names_its_command() {
        let facts = PgFacts {
            wal_level: Some(rows(&[&["replica"]])),
            replication: Some(rows(&[&["f"]])),
            ..pg_ready()
        };
        let checks = postgres_checks(&facts, "app", "orders_pub", &tables());
        assert_eq!(checks[0].status, SetupStatus::Missing);
        assert_eq!(
            checks[0].command.as_deref(),
            Some("ALTER SYSTEM SET wal_level = 'logical';")
        );
        assert_eq!(checks[1].status, SetupStatus::Missing);
        assert_eq!(
            checks[1].command.as_deref(),
            Some("ALTER ROLE \"app\" REPLICATION;")
        );
        assert_eq!(&statuses(&checks)[2..], [SetupStatus::Ok, SetupStatus::Ok]);
    }

    #[test]
    fn a_publication_that_does_not_exist_is_one_missing_check_and_the_rest_are_ok() {
        let checks = postgres_checks(&pg_ready(), "app", "no_such_pub", &tables());
        assert_eq!(
            statuses(&checks),
            [
                SetupStatus::Ok,
                SetupStatus::Ok,
                SetupStatus::Missing // no tables check: creating it names them
            ]
        );
        assert_eq!(
            checks[2].message,
            "The publication named in the connector does not exist."
        );
        assert_eq!(
            checks[2].command.as_deref(),
            Some(
                "CREATE PUBLICATION \"no_such_pub\" FOR TABLE \"public\".\"orders\", \"public\".\"items\";"
            )
        );
    }

    #[test]
    fn a_table_the_publication_lacks_is_added_by_its_command() {
        let mut facts = pg_ready();
        facts.published = Some(rows(&[&["orders_pub", "public", "orders"]]));
        let checks = postgres_checks(&facts, "app", "orders_pub", &tables());
        assert_eq!(checks[3].status, SetupStatus::Missing);
        assert_eq!(
            checks[3].command.as_deref(),
            Some("ALTER PUBLICATION \"orders_pub\" ADD TABLE \"public\".\"items\";")
        );
    }

    #[test]
    fn an_unreadable_postgres_fact_is_unchecked_never_ok() {
        let checks = postgres_checks(&PgFacts::default(), "app", "orders_pub", &tables());
        assert_eq!(
            statuses(&checks),
            [
                SetupStatus::Unchecked,
                SetupStatus::Unchecked,
                SetupStatus::Unchecked,
                SetupStatus::Unchecked
            ]
        );
        assert!(checks.iter().all(|c| c.command.is_none()));
    }

    #[test]
    fn identifiers_in_a_command_are_quoted_for_the_engine() {
        assert_eq!(pg_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(pg_table("s.t"), "\"s\".\"t\"");
        assert_eq!(mysql_string("o'brien\\"), "'o''brien\\\\'");
        assert_eq!(mssql_string("o'b"), "N'o''b'");
    }

    fn mysql_ready() -> MysqlFacts {
        MysqlFacts {
            log_bin: Some("1".to_owned()),
            binlog_format: Some("ROW".to_owned()),
            binlog_row_image: Some("FULL".to_owned()),
            grants: Some(vec![
                "GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO `cdc`@`%`".to_owned(),
                "GRANT SELECT ON `app`.* TO `cdc`@`%`".to_owned(),
            ]),
            expire_seconds: Some("2592000".to_owned()),
            expire_days: Some("0".to_owned()),
            current_user: Some("cdc@%".to_owned()),
        }
    }

    #[test]
    fn a_ready_mysql_source_passes_every_check() {
        let checks = mysql_checks(&mysql_ready(), "cdc");
        assert_eq!(checks.len(), 5);
        assert!(statuses(&checks).iter().all(|s| *s == SetupStatus::Ok));
    }

    #[test]
    fn mysql_replication_grants_need_both_privileges_on_every_database() {
        let only_client = ["GRANT REPLICATION CLIENT ON *.* TO `u`@`%`".to_owned()];
        assert!(!has_replication_grants(&only_client));
        let scoped =
            ["GRANT REPLICATION SLAVE, REPLICATION CLIENT ON `app`.* TO `u`@`%`".to_owned()];
        assert!(!has_replication_grants(&scoped));
        let all = ["GRANT ALL PRIVILEGES ON *.* TO `u`@`%` WITH GRANT OPTION".to_owned()];
        assert!(has_replication_grants(&all));
        let split = [
            "GRANT REPLICATION SLAVE ON *.* TO `u`@`%`".to_owned(),
            "GRANT REPLICATION CLIENT ON *.* TO `u`@`%`".to_owned(),
        ];
        assert!(has_replication_grants(&split));
    }

    #[test]
    fn a_login_without_replication_client_is_missing_with_its_grant() {
        let mut facts = mysql_ready();
        facts.grants = Some(vec![
            "GRANT SELECT, REPLICATION SLAVE ON *.* TO `cdc`@`%`".to_owned(),
        ]);
        let checks = mysql_checks(&facts, "cdc");
        assert_eq!(checks[3].status, SetupStatus::Missing);
        assert_eq!(
            checks[3].command.as_deref(),
            Some("GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO 'cdc'@'%';")
        );
        assert_eq!(checks[0].status, SetupStatus::Ok);
    }

    #[test]
    fn binlog_retention_is_ok_when_never_or_at_least_seven_days() {
        for (value, status) in [
            ("0", SetupStatus::Ok),
            ("604800", SetupStatus::Ok),
            ("604799", SetupStatus::Missing),
            ("3600", SetupStatus::Missing),
            ("abc", SetupStatus::Unchecked),
        ] {
            let facts = MysqlFacts {
                expire_seconds: Some(value.to_owned()),
                ..mysql_ready()
            };
            assert_eq!(mysql_retention(&facts).status, status, "{value}");
        }
        assert_eq!(
            mysql_retention(&MysqlFacts::default()).status,
            SetupStatus::Unchecked
        );
    }

    /// Slice 4 review SHOULD-FIX 6 and 8, and the privilege-list NIT.
    #[test]
    fn legacy_expiry_days_and_the_servers_own_account_are_honoured() {
        let days = |seconds: &str, days: &str| MysqlFacts {
            expire_seconds: Some(seconds.to_owned()),
            expire_days: Some(days.to_owned()),
            ..mysql_ready()
        };
        assert_eq!(mysql_retention(&days("0", "0")).status, SetupStatus::Ok);
        assert_eq!(
            mysql_retention(&days("0", "3")).status,
            SetupStatus::Missing
        );
        assert_eq!(mysql_retention(&days("0", "30")).status, SetupStatus::Ok);
        assert_eq!(
            mysql_retention(&days("2592000", "1")).status,
            SetupStatus::Ok
        );
        let unread = MysqlFacts {
            expire_days: None,
            ..mysql_ready()
        };
        assert_eq!(mysql_retention(&unread).status, SetupStatus::Unchecked);

        let facts = MysqlFacts {
            grants: Some(vec!["GRANT SELECT ON *.* TO `o'b`@`10.0.%`".to_owned()]),
            current_user: Some("o'b@10.0.%".to_owned()),
            ..mysql_ready()
        };
        let checks = mysql_checks(&facts, "dial_user");
        assert_eq!(
            checks[3].command.as_deref(),
            Some("GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO 'o''b'@'10.0.%';")
        );
        let table_named_like_a_privilege =
            ["GRANT SELECT ON `REPLICATION SLAVE REPLICATION CLIENT`.* TO `u`@`%`".to_owned()];
        assert!(!has_replication_grants(&table_named_like_a_privilege));
    }

    #[test]
    fn an_agent_query_that_lists_no_agent_is_unchecked_not_stopped() {
        let facts = MssqlFacts {
            cdc_enabled: Some(true),
            tables: Some(Vec::new()),
            agent: AgentState::NotListed,
        };
        let checks = mssql_checks(&facts, &[]);
        assert_eq!(checks[1].status, SetupStatus::Unchecked);
        assert!(checks[1].message.contains("lists no SQL Server Agent"));
    }

    #[test]
    fn the_wrong_binlog_format_and_row_image_each_name_their_command() {
        let facts = MysqlFacts {
            log_bin: Some("0".to_owned()),
            binlog_format: Some("MIXED".to_owned()),
            binlog_row_image: Some("MINIMAL".to_owned()),
            ..mysql_ready()
        };
        let checks = mysql_checks(&facts, "cdc");
        assert_eq!(&statuses(&checks)[..3], [SetupStatus::Missing; 3]);
        assert_eq!(
            checks[1].command.as_deref(),
            Some("SET PERSIST binlog_format = 'ROW';")
        );
        assert_eq!(
            checks[2].command.as_deref(),
            Some("SET PERSIST binlog_row_image = 'FULL';")
        );
    }

    #[test]
    fn sql_server_checks_each_selected_table_and_never_passes_an_unread_agent() {
        let facts = MssqlFacts {
            cdc_enabled: Some(true),
            tables: Some(vec![
                ("dbo".to_owned(), "orders".to_owned(), true),
                ("dbo".to_owned(), "items".to_owned(), true),
                ("dbo".to_owned(), "extra".to_owned(), false),
            ]),
            agent: AgentState::Unread,
        };
        let selected = vec![
            "dbo.orders".to_owned(),
            "dbo.items".to_owned(),
            "dbo.extra".to_owned(),
        ];
        let checks = mssql_checks(&facts, &selected);
        assert_eq!(
            statuses(&checks),
            [
                SetupStatus::Ok,
                SetupStatus::Ok,
                SetupStatus::Ok,
                SetupStatus::Missing,
                SetupStatus::Unchecked
            ]
        );
        assert_eq!(
            checks[3].command.as_deref(),
            Some(
                "EXEC sys.sp_cdc_enable_table @source_schema = N'dbo', @source_name = N'extra', @role_name = NULL;"
            )
        );
        assert!(checks[4].message.contains("VIEW SERVER STATE"));
    }

    #[test]
    fn a_sql_server_without_cdc_or_agent_says_so() {
        let facts = MssqlFacts {
            cdc_enabled: Some(false),
            tables: Some(Vec::new()),
            agent: AgentState::NotRunning,
        };
        let checks = mssql_checks(&facts, &["dbo.orders".to_owned()]);
        assert_eq!(
            statuses(&checks),
            [
                SetupStatus::Missing,
                SetupStatus::Missing,
                SetupStatus::Missing
            ]
        );
        assert_eq!(
            checks[0].command.as_deref(),
            Some("EXEC sys.sp_cdc_enable_db;")
        );
        assert!(checks[2].command.is_none());
        let unread = mssql_checks(&MssqlFacts::default(), &["dbo.orders".to_owned()]);
        assert_eq!(unread[0].status, SetupStatus::Unchecked);
        assert_eq!(unread[1].status, SetupStatus::Unchecked);
    }

    // ── MongoDB (SRC-4 Amendment 8) ──

    fn mongo_dial(user: &str) -> CdcMongoDial {
        serde_json::from_value(serde_json::json!({
            "driver": "mongodb", "hosts": ["m.internal:27017"], "database": "shop",
            "user": user, "replicaSet": "rs0"
        }))
        .unwrap()
    }

    fn by_setting<'a>(checks: &'a [SetupCheck], setting: &str) -> &'a SetupCheck {
        checks.iter().find(|c| c.setting == setting).unwrap()
    }

    fn all_ok() -> MongoSetupFacts {
        MongoSetupFacts {
            replica_set: Some(ReplicaSetFact::Matches),
            primary: Some(true),
            change_stream: Some(ChangeStreamFact::Granted),
        }
    }

    #[test]
    fn mongo_checks_are_all_ok_when_every_fact_is_good() {
        let checks = mongo_checks(Some(&all_ok()), &mongo_dial("reader"));
        assert_eq!(
            checks
                .iter()
                .map(|c| c.setting.as_str())
                .collect::<Vec<_>>(),
            ["replicaSet", "primary", "changeStream"]
        );
        assert!(
            checks
                .iter()
                .all(|c| c.status == SetupStatus::Ok && c.command.is_none())
        );
    }

    #[test]
    fn a_replica_set_that_differs_or_is_none_is_missing_with_its_fixed_sentence_and_no_command() {
        let dial = mongo_dial("reader");
        for (fact, sentence) in [
            (
                ReplicaSetFact::Differs,
                "The server's replica set has another name. Use the name in `rs.status().set`.",
            ),
            (
                ReplicaSetFact::None,
                "The server is not a replica set member; change streams need a replica set.",
            ),
        ] {
            let facts = MongoSetupFacts {
                replica_set: Some(fact),
                ..all_ok()
            };
            let checks = mongo_checks(Some(&facts), &dial);
            let check = by_setting(&checks, "replicaSet");
            assert_eq!(
                (check.status, check.message.as_str(), &check.command),
                (SetupStatus::Missing, sentence, &None)
            );
            // The other two are unaffected.
            assert_eq!(by_setting(&checks, "primary").status, SetupStatus::Ok);
            assert_eq!(by_setting(&checks, "changeStream").status, SetupStatus::Ok);
        }
    }

    #[test]
    fn a_host_that_is_not_the_primary_is_missing_with_no_command() {
        let facts = MongoSetupFacts {
            primary: Some(false),
            ..all_ok()
        };
        let checks = mongo_checks(Some(&facts), &mongo_dial("reader"));
        let check = by_setting(&checks, "primary");
        assert_eq!(check.status, SetupStatus::Missing);
        assert_eq!(
            check.message,
            "This host is not the primary. The connector reads change streams through the one host it names, so name the primary."
        );
        assert!(check.command.is_none());
    }

    #[test]
    fn a_missing_change_stream_privilege_carries_a_grant_built_from_the_dial() {
        let facts = MongoSetupFacts {
            change_stream: Some(ChangeStreamFact::Missing),
            ..all_ok()
        };
        let checks = mongo_checks(Some(&facts), &mongo_dial("reader"));
        let check = by_setting(&checks, "changeStream");
        assert_eq!(check.status, SetupStatus::Missing);
        assert_eq!(
            check.command.as_deref(),
            Some(
                "db.getSiblingDB(\"shop\").grantRolesToUser(\"reader\", [{ role: \"read\", db: \"shop\" }])"
            )
        );
    }

    #[test]
    fn the_grants_user_is_quoted_as_a_json_string_never_by_hand() {
        // A quote and a backslash in the user cannot end the string literal.
        let facts = MongoSetupFacts {
            change_stream: Some(ChangeStreamFact::Missing),
            ..all_ok()
        };
        let checks = mongo_checks(Some(&facts), &mongo_dial("a\"b\\c"));
        let command = by_setting(&checks, "changeStream").command.clone().unwrap();
        assert!(
            command.contains(r#"grantRolesToUser("a\"b\\c","#),
            "{command}"
        );
    }

    #[test]
    fn absent_facts_are_unchecked_never_ok() {
        let dial = mongo_dial("reader");
        for facts in [None, Some(&MongoSetupFacts::default())] {
            let checks = mongo_checks(facts, &dial);
            assert_eq!(checks.len(), 3);
            assert!(
                checks
                    .iter()
                    .all(|c| c.status == SetupStatus::Unchecked && c.command.is_none())
            );
            assert!(checks.iter().all(|c| c.message == UNREAD));
        }
        // One fact absent leaves only that check unchecked.
        let facts = MongoSetupFacts {
            primary: None,
            ..all_ok()
        };
        let checks = mongo_checks(Some(&facts), &dial);
        assert_eq!(
            by_setting(&checks, "primary").status,
            SetupStatus::Unchecked
        );
        assert_eq!(by_setting(&checks, "replicaSet").status, SetupStatus::Ok);
    }

    // ── Oracle ──

    fn oracle_ready() -> OracleSetupFacts {
        OracleSetupFacts {
            log_mode: Some(OracleLogMode::Archivelog),
            supplemental_min: Some(OracleSupplementalMin::Yes),
            supplemental_all: Some(false),
            tables: Some(
                [("SHOP.ORDERS".to_owned(), OracleTableFact::AllColumns)]
                    .into_iter()
                    .collect(),
            ),
            missing_privileges: Some(Vec::new()),
            encryption_required: None,
        }
    }

    fn oracle_tables() -> Vec<String> {
        vec!["SHOP.ORDERS".to_owned()]
    }

    #[test]
    fn oracle_checks_are_all_ok_when_every_fact_is_good() {
        let checks = oracle_checks(Some(&oracle_ready()), &oracle_tables(), "c##dbzuser");
        let settings: Vec<&str> = checks.iter().map(|c| c.setting.as_str()).collect();
        assert_eq!(
            settings,
            [
                "ARCHIVELOG",
                "Supplemental logging",
                "All-column logging on SHOP.ORDERS",
                "LogMiner privileges"
            ]
        );
        assert!(
            checks
                .iter()
                .all(|c| c.status == SetupStatus::Ok && c.command.is_none())
        );
    }

    #[test]
    fn oracle_missing_settings_say_so_with_quoted_commands() {
        let facts = OracleSetupFacts {
            log_mode: Some(OracleLogMode::Noarchivelog),
            supplemental_min: Some(OracleSupplementalMin::No),
            tables: Some(
                [
                    ("SHOP.PLAIN".to_owned(), OracleTableFact::WithoutAllColumns),
                    ("SHOP.TYPO".to_owned(), OracleTableFact::NotFound),
                ]
                .into_iter()
                .collect(),
            ),
            missing_privileges: Some(vec![
                OraclePrivilege::Logmining,
                OraclePrivilege::CreateSession,
            ]),
            ..OracleSetupFacts::default()
        };
        let tables = vec!["SHOP.PLAIN".to_owned(), "SHOP.TYPO".to_owned()];
        let checks = oracle_checks(Some(&facts), &tables, "c##db\"z");
        assert_eq!(checks[0].status, SetupStatus::Missing);
        assert_eq!(
            checks[0].command, None,
            "ARCHIVELOG has no one-line command"
        );
        assert!(checks[0].message.contains("STARTUP MOUNT"));
        assert_eq!(
            checks[1].command.as_deref(),
            Some("ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;")
        );
        assert_eq!(
            checks[2].command.as_deref(),
            Some("ALTER TABLE \"SHOP\".\"PLAIN\" ADD SUPPLEMENTAL LOG DATA (ALL) COLUMNS;")
        );
        assert_eq!(checks[3].status, SetupStatus::Missing);
        assert_eq!(
            checks[3].command, None,
            "a name nobody found has no command"
        );
        // The user is upper-cased and its quotes are doubled; the list is in a fixed order.
        assert_eq!(
            checks[4].command.as_deref(),
            Some("GRANT CREATE SESSION, LOGMINING TO \"C##DB\"\"Z\" CONTAINER=ALL;")
        );
    }

    #[test]
    fn oracle_database_level_all_column_logging_makes_every_table_ok() {
        let facts = OracleSetupFacts {
            supplemental_all: Some(true),
            tables: Some(
                [("SHOP.PLAIN".to_owned(), OracleTableFact::WithoutAllColumns)]
                    .into_iter()
                    .collect(),
            ),
            ..oracle_ready()
        };
        let checks = oracle_checks(Some(&facts), &["SHOP.PLAIN".to_owned()], "u");
        assert_eq!(checks[2].status, SetupStatus::Ok);
    }

    #[test]
    fn oracle_unread_facts_and_an_unlisted_table_are_unchecked_never_ok() {
        for facts in [None, Some(&OracleSetupFacts::default())] {
            let checks = oracle_checks(facts, &oracle_tables(), "u");
            assert!(
                checks.iter().all(|c| c.status == SetupStatus::Unchecked),
                "{checks:?}"
            );
        }
        // A table chosen after the test started is not in the posted map.
        let checks = oracle_checks(
            Some(&oracle_ready()),
            &["SHOP.ORDERS".to_owned(), "SHOP.NEW".to_owned()],
            "u",
        );
        assert_eq!(checks[2].status, SetupStatus::Ok);
        assert_eq!(checks[3].status, SetupStatus::Unchecked);
    }

    #[test]
    fn oracle_a_server_that_requires_encryption_makes_every_check_unchecked_with_the_fixed_line() {
        let facts = OracleSetupFacts {
            encryption_required: Some(true),
            ..OracleSetupFacts::default()
        };
        let checks = oracle_checks(Some(&facts), &oracle_tables(), "u");
        assert_eq!(checks.len(), 4);
        assert!(
            checks
                .iter()
                .all(|c| c.status == SetupStatus::Unchecked
                    && c.message == ORACLE_ENCRYPTION_REQUIRED)
        );
    }

    #[test]
    fn oracle_a_failed_connectors_trace_explains_only_the_two_messages_debezium_throws() {
        let archive = "org.apache.kafka.connect.errors.ConnectException: io.debezium.DebeziumException: \
            The Oracle server is not configured to use a archive log LOG_MODE, which is required for this \
            connector to work properly. Change the Oracle configuration to use a LOG_MODE=ARCHIVELOG and \
            restart the connector.";
        let checks = oracle_trace_checks(archive);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].setting, "ARCHIVELOG");
        assert_eq!(checks[0].status, SetupStatus::Missing);
        let supplemental = "io.debezium.DebeziumException: Supplemental logging not properly configured. \
            Use: ALTER DATABASE ADD SUPPLEMENTAL LOG DATA";
        let checks = oracle_trace_checks(supplemental);
        assert_eq!(checks.len(), 1);
        assert_eq!(
            checks[0].command.as_deref(),
            Some("ALTER DATABASE ADD SUPPLEMENTAL LOG DATA;")
        );
        let both = oracle_trace_checks(&format!("{archive}\n{supplemental}"));
        assert_eq!(both.len(), 2);
        // Anything else explains nothing, and none of its text is carried.
        assert_eq!(
            oracle_trace_checks("ORA-01017: invalid username/password SECRET_USER").len(),
            0
        );
    }

    #[test]
    fn oracle_a_per_table_grant_alternative_is_unchecked_not_missing() {
        let facts = OracleSetupFacts {
            missing_privileges: Some(vec![OraclePrivilege::SelectAnyTable]),
            ..oracle_ready()
        };
        let checks = oracle_checks(Some(&facts), &oracle_tables(), "u");
        assert_eq!(checks[3].status, SetupStatus::Unchecked);
        assert!(checks[3].command.is_none());
    }
}
