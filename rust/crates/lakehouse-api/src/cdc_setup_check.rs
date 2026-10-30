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

use lakehouse_store::connectors::{SetupCheck, SetupStatus};

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
}

fn mysql_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "''"))
}

/// The `MySQL` checks: the binary log, its format and row image, the login's
/// replication privileges and the binlog retention.
#[must_use]
pub fn mysql_checks(facts: &MysqlFacts, user: &str) -> Vec<SetupCheck> {
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
        mysql_grants(facts, user),
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
            format!("SET GLOBAL {setting} = '{wanted}';"),
        ),
    }
}

/// Whether the grants hold the two replication privileges on every database
/// (`ON *.*`), directly or through `ALL PRIVILEGES`.
fn has_replication_grants(grants: &[String]) -> bool {
    let global: Vec<String> = grants
        .iter()
        .filter(|g| g.contains(" ON *.* "))
        .map(|g| g.to_ascii_uppercase())
        .collect();
    if global.iter().any(|g| g.contains("ALL PRIVILEGES")) {
        return true;
    }
    let holds = |privilege: &str| global.iter().any(|g| g.contains(privilege));
    holds("REPLICATION SLAVE") && holds("REPLICATION CLIENT")
}

fn mysql_grants(facts: &MysqlFacts, user: &str) -> SetupCheck {
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
            format!(
                "GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO {}@'%';",
                mysql_string(user)
            ),
        ),
    }
}

fn mysql_retention(facts: &MysqlFacts) -> SetupCheck {
    const SETTING: &str = "binlog retention";
    let seconds = facts
        .expire_seconds
        .as_deref()
        .and_then(|v| v.trim().parse::<i64>().ok());
    match seconds {
        None => unchecked(SETTING, UNREAD),
        Some(0) => ok(SETTING, "Binary logs never expire."),
        Some(s) if s >= MIN_BINLOG_RETENTION_SECONDS => {
            ok(SETTING, "Binary logs are kept for at least 7 days.")
        }
        Some(_) => missing(
            SETTING,
            "Binary logs expire in under 7 days, so a connector stopped for longer cannot catch up.",
            format!("SET GLOBAL binlog_expire_logs_seconds = {MIN_BINLOG_RETENTION_SECONDS};"),
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
    /// The service is not running or not listed.
    NotRunning,
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
    let tracked = listed
        .iter()
        .any(|(s, t, tracked)| *tracked && s == schema && t == table);
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn rows(values: &[&[&str]]) -> Option<Rows> {
        Some(
            values
                .iter()
                .map(|row| row.iter().map(|c| Some((*c).to_owned())).collect())
                .collect(),
        )
    }

    fn statuses(checks: &[SetupCheck]) -> Vec<SetupStatus> {
        checks.iter().map(|c| c.status).collect()
    }

    fn pg_ready() -> PgFacts {
        PgFacts {
            wal_level: rows(&[&["logical"]]),
            replication: rows(&[&["t"]]),
            publications: rows(&[&["other_pub"], &["orders_pub"]]),
            published: rows(&[
                &["orders_pub", "public", "orders"],
                &["orders_pub", "public", "items"],
                &["other_pub", "public", "extra"],
            ]),
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
            wal_level: rows(&[&["replica"]]),
            replication: rows(&[&["f"]]),
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
        facts.published = rows(&[&["orders_pub", "public", "orders"]]);
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
            Some("SET GLOBAL binlog_format = 'ROW';")
        );
        assert_eq!(
            checks[2].command.as_deref(),
            Some("SET GLOBAL binlog_row_image = 'FULL';")
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
}
