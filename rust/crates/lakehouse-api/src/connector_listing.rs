//! Table discovery that the API runs itself (`SRC-13` decisions D1 to D10): a
//! SQL source's schemas, tables and columns, an object-storage bucket's files
//! (by glob) with a CSV or Parquet column preview, and a REST connector's
//! declared endpoints. The wire shapes are `lakehouse_store::connector_discovery`'s.
//!
//! `MongoDB`, Kafka, SFTP and Oracle (and the column preview of REST, `MongoDB`,
//! Kafka and SFTP) are read by the orchestrator, which holds their drivers
//! (D1); [`dispatch`] says so with [`Dispatch::Orchestrator`] and the route
//! launches the job (`routes::connector_discovery`).
//!
//! # What this module protects
//!
//! - **Names first, columns on demand (D9).** A list asks the source for names
//!   only (`information_schema.tables`, one cheap query), never for every
//!   column of every table; the columns of one table are a second call
//!   ([`SqlOp::Columns`]). Both are bounded by [`LIST_TIMEOUT`] (120 s, not the
//!   5-second test limit) and a row limit, so a schema of 10,000 tables is one
//!   query that returns 10,000 short strings.
//! - **Base tables only (D8).** Every listing query filters
//!   `table_type = 'BASE TABLE'`, as the loader does.
//! - **Values are bound (AGENTS.md).** The schema, the table, the search text
//!   and the row limit are bind parameters in every query; `format!` is not
//!   used on any SQL here. A search text is LIKE-escaped ([`like_escape`]) so
//!   `%` and `_` in it match themselves.
//! - **Fail closed, the same order as the connection test.** The address check
//!   ([`connector_probe::resolve_checked`]) runs before the credential is
//!   resolved and before any dial, and the dial goes to the approved address
//!   (`SEC-15`).
//! - **Principle 4.** A failure is an [`ErrorClass`] chosen from the error's
//!   TYPE and CODE; no driver, `object_store` or source text is read into a
//!   response (the log gets the class and nothing else).
//!
//! # Globs (D2)
//!
//! A file glob is matched against the object's FULL key, as a source object's
//! `name` is the full key. `*` matches any run of characters except `/`, `**`
//! any run including `/`, `?` one character except `/`; anything else matches
//! itself. The default is `**` (everything). The connector's `prefix`, when it
//! has one, narrows the listing as well: only keys that start with it are shown.
//! The matcher is a linear automaton, not backtracking, so a hostile pattern
//! cannot take exponential time; the pattern is at most [`MAX_GLOB_CHARS`]
//! characters and a bucket scan stops at [`MAX_SCANNED`] keys.

use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use lakehouse_core::secret::DynSecretResolver;
use lakehouse_store::connector_discovery::{
    Answer, ErrorClass, Kind, ListedColumn, ListedObject, Listing, MAX_COLUMNS, MAX_NAME_CHARS,
    ObjectKind, Preview,
};
use lakehouse_store::connectors::ConnectorDialInfo;
use lakehouse_store::ingest_spec::{Dial, FilesDial, FilesFormat, FilesProtocol, SqlDriver};
use object_store::aws::AmazonS3;
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt};
use sqlx::mysql::MySqlPoolOptions;
use sqlx::postgres::PgPoolOptions;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime};
use tokio_util::compat::TokioAsyncWriteCompatExt;

use crate::connector_probe::{self, Approved, DialTarget};
use crate::internal_hosts::InternalHosts;
use crate::{upload_parquet, upload_parse};

/// The limit of one listing or one preview, connect included (`SRC-13`
/// decision D9: "the listing call's own limit is 120 s").
pub const LIST_TIMEOUT: Duration = Duration::from_secs(120);

/// The most characters of a glob.
pub const MAX_GLOB_CHARS: usize = 256;

/// The most bucket keys one listing reads before it gives up with
/// `too_many_objects`; the matches it keeps are bounded separately by the
/// request's limit.
pub const MAX_SCANNED: usize = 100_000;

/// The CSV preview reads this many bytes of the file's head.
const CSV_HEAD_BYTES: u64 = 1024 * 1024;

/// The CSV preview shows the header and at most this many rows' types (D6).
const CSV_PREVIEW_ROWS: usize = 1000;

/// A Parquet file is read whole to read its footer
/// ([`upload_parquet::inspect`] takes the bytes); a larger one is refused.
const PARQUET_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// What a listing or preview request asks, after the route validated it.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// A SQL source's schema; empty lists the schemas (D7).
    pub schema: String,
    /// A file glob; empty means everything (D2).
    pub glob: String,
    /// Part of a name to search for, case-insensitive; empty means no search.
    pub search: String,
    /// The most objects to return, at most [`MAX_OBJECTS`].
    pub limit: usize,
    /// The object a preview is about; empty for a list.
    pub object: String,
}

/// Who answers a request.
#[derive(Debug)]
pub enum Dispatch {
    /// The API answered (or refused) itself.
    Done(Answer),
    /// The orchestrator must read the source (D1): launch `discover_job`.
    Orchestrator,
}

// ── Classifying a failure (principle 4) ───────────────────────────────────

/// The class of a `sqlx` error, from its type and its code, never its text.
pub(crate) fn class_of_sqlx(err: &sqlx::Error) -> ErrorClass {
    match err {
        sqlx::Error::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => ErrorClass::Timeout,
        sqlx::Error::Io(_) | sqlx::Error::Tls(_) => ErrorClass::Unreachable,
        sqlx::Error::PoolTimedOut => ErrorClass::Timeout,
        sqlx::Error::Database(db) => {
            let mysql_number = db
                .try_downcast_ref::<sqlx::mysql::MySqlDatabaseError>()
                .map(sqlx::mysql::MySqlDatabaseError::number);
            match (db.code().as_deref(), mysql_number) {
                // 28P01 invalid_password, 28000 invalid_authorization_specification
                // (also MySQL's SQLSTATE for 1045).
                (Some("28P01" | "28000"), _) | (_, Some(1045)) => ErrorClass::SignInRefused,
                // 42501 insufficient_privilege; MySQL 1044/1142/1143/1227/1370.
                (Some("42501"), _) | (_, Some(1044 | 1142 | 1143 | 1227 | 1370)) => {
                    ErrorClass::NoPermission
                }
                // 57014 query_canceled (a statement timeout).
                (Some("57014"), _) => ErrorClass::Timeout,
                // 08xxx connection exceptions, 57P03 cannot_connect_now, 53300
                // too_many_connections.
                (Some(code), _) if code.starts_with("08") || code == "57P03" || code == "53300" => {
                    ErrorClass::Unreachable
                }
                _ => ErrorClass::Failed,
            }
        }
        _ => ErrorClass::Failed,
    }
}

/// The class of a `tiberius` error, from its type and its server error number.
pub(crate) fn class_of_tiberius(err: &tiberius::error::Error) -> ErrorClass {
    match err {
        tiberius::error::Error::Io { kind, .. } if *kind == std::io::ErrorKind::TimedOut => {
            ErrorClass::Timeout
        }
        tiberius::error::Error::Io { .. } | tiberius::error::Error::Tls(_) => {
            ErrorClass::Unreachable
        }
        tiberius::error::Error::Server(_) => match err.code() {
            // 18456: login failed.
            Some(18_456) => ErrorClass::SignInRefused,
            // 229/230 permission denied on an object/column, 262, 297, 300, 916.
            Some(229 | 230 | 262 | 297 | 300 | 916) => ErrorClass::NoPermission,
            _ => ErrorClass::Failed,
        },
        _ => ErrorClass::Failed,
    }
}

/// The class of an `object_store` error. `connector_probe`'s classifier reads
/// only the error's type (never its `Display`, which carries the response body)
/// and answers a fixed phrase; this maps those phrases.
pub(crate) fn class_of_object_store(err: &object_store::Error) -> ErrorClass {
    match connector_probe::classify_object_store_error(err) {
        "permission denied" => ErrorClass::NoPermission,
        "authentication failed" => ErrorClass::SignInRefused,
        "not supported" => ErrorClass::NotSupported,
        "connection refused" => ErrorClass::Unreachable,
        "timed out" => ErrorClass::Timeout,
        _ => ErrorClass::Failed,
    }
}

/// The class of a refusal from [`connector_probe::resolve_checked`]: its message
/// is a fixed sentence of the product's own. The address check's refusal starts
/// "refusing to dial"; anything else is a name that did not resolve.
fn class_of_address_error(message: &str) -> ErrorClass {
    if message.starts_with("refusing to dial") {
        ErrorClass::AddressRefused
    } else {
        ErrorClass::Unreachable
    }
}

/// Log the class of a failure and nothing else (no error text), then return it.
fn logged(what: &'static str, class: ErrorClass) -> ErrorClass {
    tracing::warn!(class = class.as_str(), "SRC-13: {what} failed");
    class
}

// ── Pure helpers ──────────────────────────────────────────────────────────

/// `text` as a LIKE operand that matches itself: `\`, `%`, `_` and `[` are
/// escaped with a backslash (the escape character of every query below).
pub(crate) fn like_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '%' | '_' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The row limit to fetch: one more than the limit asked, so a cut is seen.
fn fetch_limit(limit: usize) -> i64 {
    i64::try_from(limit.saturating_add(1)).unwrap_or(i64::MAX)
}

/// Cut `names` at `limit`, saying whether anything was left over.
fn bounded(mut names: Vec<String>, limit: usize) -> (Vec<String>, bool) {
    let truncated = names.len() > limit;
    names.truncate(limit);
    (names, truncated)
}

/// `schema.table` split at its first dot, both parts non-empty.
pub(crate) fn split_object(object: &str) -> Option<(&str, &str)> {
    object
        .split_once('.')
        .filter(|(schema, table)| !schema.is_empty() && !table.is_empty())
}

/// One token of a compiled glob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Lit(char),
    /// `?`: one character except `/`.
    One,
    /// `*`: a run of characters except `/`.
    Star,
    /// `**`: a run of any characters.
    Deep,
}

/// A compiled file glob (module docs, "Globs").
#[derive(Debug, Clone)]
pub struct Glob {
    tokens: Vec<Tok>,
    literal_prefix: String,
}

impl Glob {
    /// Compile `glob`; empty means everything. `None` for a glob longer than
    /// [`MAX_GLOB_CHARS`] or holding a control character.
    #[must_use]
    pub fn parse(glob: &str) -> Option<Self> {
        let glob = if glob.is_empty() { "**" } else { glob };
        if glob.chars().count() > MAX_GLOB_CHARS || glob.chars().any(char::is_control) {
            return None;
        }
        let chars: Vec<char> = glob.chars().collect();
        let mut tokens = Vec::with_capacity(chars.len());
        let mut index = 0;
        while let Some(&c) = chars.get(index) {
            match c {
                '*' if chars.get(index + 1) == Some(&'*') => {
                    tokens.push(Tok::Deep);
                    index += 2;
                }
                '*' => {
                    tokens.push(Tok::Star);
                    index += 1;
                }
                '?' => {
                    tokens.push(Tok::One);
                    index += 1;
                }
                other => {
                    tokens.push(Tok::Lit(other));
                    index += 1;
                }
            }
        }
        let literal_prefix = tokens
            .iter()
            .map_while(|tok| match tok {
                Tok::Lit(c) => Some(*c),
                _ => None,
            })
            .collect();
        Some(Self {
            tokens,
            literal_prefix,
        })
    }

    /// The text before the first wildcard.
    #[must_use]
    pub fn literal_prefix(&self) -> &str {
        &self.literal_prefix
    }

    /// Whether the whole of `text` matches. A set of positions advanced one
    /// character at a time: linear in the text times the pattern.
    #[must_use]
    pub fn matches(&self, text: &str) -> bool {
        let count = self.tokens.len();
        let mut current = vec![false; count + 1];
        current[0] = true;
        self.close(&mut current);
        for ch in text.chars() {
            let mut next = vec![false; count + 1];
            for (at, tok) in self.tokens.iter().enumerate() {
                if !current[at] {
                    continue;
                }
                match tok {
                    Tok::Lit(lit) if *lit == ch => next[at + 1] = true,
                    Tok::One if ch != '/' => next[at + 1] = true,
                    Tok::Star if ch != '/' => next[at] = true,
                    Tok::Deep => next[at] = true,
                    _ => {}
                }
            }
            self.close(&mut next);
            current = next;
        }
        current[count]
    }

    /// A `*` or `**` may match nothing: a position before one is also a position
    /// after it.
    fn close(&self, states: &mut [bool]) {
        for (at, tok) in self.tokens.iter().enumerate() {
            if states[at] && matches!(tok, Tok::Star | Tok::Deep) {
                states[at + 1] = true;
            }
        }
    }
}

/// The bucket "directory" to list for `connector_prefix` and a glob's literal
/// prefix: the part before the last `/` of whichever is the longer, when one
/// contains the other. `None` when the two disagree (nothing can match).
/// `object_store` lists by whole path segments, so a partial last segment is
/// left to the filters.
pub(crate) fn listing_directory(connector_prefix: &str, glob_literal: &str) -> Option<String> {
    let longer = if connector_prefix.starts_with(glob_literal) {
        connector_prefix
    } else if glob_literal.starts_with(connector_prefix) {
        glob_literal
    } else {
        return None;
    };
    Some(
        longer
            .rfind('/')
            .map_or("", |slash| &longer[..slash])
            .to_owned(),
    )
}

fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    needle.is_empty() || haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// A CSV header cell as a column name: control characters dropped, cut at
/// [`MAX_NAME_CHARS`]; a blank cell is `column_N`.
fn header_name(index: usize, cell: &str) -> String {
    let cleaned: String = cell
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    if cleaned.trim().is_empty() {
        format!("column_{}", index + 1)
    } else {
        cleaned
    }
}

/// The type a column's sampled values fit (D6): `integer`, `number`,
/// `boolean`, `date`, `timestamp`, else `string`. Blank values are skipped; a
/// column with no value is `string`. The caller marks the result inferred.
pub(crate) fn infer_type<'a>(values: impl Iterator<Item = &'a str>) -> &'static str {
    let date = format_description!("[year]-[month]-[day]");
    let stamp = format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    let (mut integer, mut number, mut boolean, mut dates, mut stamps) =
        (true, true, true, true, true);
    let mut any = false;
    for value in values.map(str::trim).filter(|value| !value.is_empty()) {
        any = true;
        integer &= value.parse::<i64>().is_ok();
        number &= value.parse::<f64>().is_ok_and(f64::is_finite);
        boolean &= value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false");
        dates &= Date::parse(value, &date).is_ok();
        stamps &= OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .is_ok()
            || PrimitiveDateTime::parse(value, &stamp).is_ok();
    }
    if !any {
        "string"
    } else if integer {
        "integer"
    } else if number {
        "number"
    } else if boolean {
        "boolean"
    } else if dates {
        "date"
    } else if stamps {
        "timestamp"
    } else {
        "string"
    }
}

/// The columns of a CSV file's first bytes: the header cells, each typed from
/// the first [`CSV_PREVIEW_ROWS`] rows (D6), all marked inferred. `truncated` says
/// `head` is only the start of the file.
pub(crate) fn csv_preview(head: &[u8], truncated: bool) -> Result<Preview, ErrorClass> {
    let read = upload_parse::preview(
        head,
        truncated,
        upload_parse::Overrides::default(),
        CSV_PREVIEW_ROWS,
    );
    if read.columns.is_empty() || read.columns.len() > MAX_COLUMNS {
        return Err(ErrorClass::Failed);
    }
    let columns = read
        .columns
        .iter()
        .enumerate()
        .map(|(index, cell)| ListedColumn {
            name: header_name(index, cell),
            type_name: infer_type(
                read.rows
                    .iter()
                    .filter_map(|row| row.get(index).map(String::as_str)),
            )
            .to_owned(),
            inferred: true,
        })
        .collect();
    Ok(Preview {
        columns,
        primary_key: None,
    })
}

// ── SQL ───────────────────────────────────────────────────────────────────

/// What a SQL request asks of the source.
enum SqlOp<'a> {
    /// The schemas the connector's account can see (D7).
    Schemas { search: &'a str, limit: usize },
    /// The base-table names of one schema (D8, D9).
    Tables {
        schema: &'a str,
        search: &'a str,
        limit: usize,
    },
    /// The columns and primary key of one table, on demand (D9).
    Columns { schema: &'a str, table: &'a str },
}

/// What a SQL request returns.
#[derive(Debug)]
enum SqlOut {
    Names { names: Vec<String>, truncated: bool },
    Columns(Preview),
}

const PG_SCHEMAS: &str = "SELECT schema_name::text FROM information_schema.schemata \
    WHERE schema_name NOT IN ('information_schema', 'pg_catalog') \
      AND left(schema_name, 3) <> 'pg_' \
      AND ($1 = '' OR schema_name ILIKE '%' || $1 || '%' ESCAPE '\\') \
    ORDER BY schema_name LIMIT $2";

const PG_TABLES: &str = "SELECT table_name::text FROM information_schema.tables \
    WHERE table_schema = $1 AND table_type = 'BASE TABLE' \
      AND ($2 = '' OR table_name ILIKE '%' || $2 || '%' ESCAPE '\\') \
    ORDER BY table_name LIMIT $3";

const PG_COLUMNS: &str = "SELECT column_name::text, data_type::text \
    FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 \
    ORDER BY ordinal_position";

const PG_KEY: &str = "SELECT kcu.column_name::text \
    FROM information_schema.table_constraints tc \
    JOIN information_schema.key_column_usage kcu \
      ON kcu.constraint_schema = tc.constraint_schema \
     AND kcu.constraint_name = tc.constraint_name \
     AND kcu.table_name = tc.table_name \
    WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = $1 AND tc.table_name = $2 \
    ORDER BY kcu.ordinal_position";

const MYSQL_SCHEMAS: &str = "SELECT CAST(schema_name AS CHAR CHARACTER SET utf8mb4) \
    FROM information_schema.schemata \
    WHERE schema_name NOT IN ('information_schema', 'mysql', 'performance_schema', 'sys') \
      AND (? = '' OR schema_name LIKE CONCAT('%', ?, '%')) \
    ORDER BY schema_name LIMIT ?";

const MYSQL_TABLES: &str = "SELECT CAST(table_name AS CHAR CHARACTER SET utf8mb4) \
    FROM information_schema.tables \
    WHERE table_schema = ? AND table_type = 'BASE TABLE' \
      AND (? = '' OR table_name LIKE CONCAT('%', ?, '%')) \
    ORDER BY table_name LIMIT ?";

const MYSQL_COLUMNS: &str = "SELECT CAST(column_name AS CHAR CHARACTER SET utf8mb4), \
    CAST(data_type AS CHAR CHARACTER SET utf8mb4) \
    FROM information_schema.columns WHERE table_schema = ? AND table_name = ? \
    ORDER BY ordinal_position";

const MYSQL_KEY: &str = "SELECT CAST(kcu.column_name AS CHAR CHARACTER SET utf8mb4) \
    FROM information_schema.table_constraints tc \
    JOIN information_schema.key_column_usage kcu \
      ON kcu.constraint_schema = tc.constraint_schema \
     AND kcu.constraint_name = tc.constraint_name \
     AND kcu.table_schema = tc.table_schema \
     AND kcu.table_name = tc.table_name \
    WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = ? AND tc.table_name = ? \
    ORDER BY kcu.ordinal_position";

const MSSQL_SCHEMAS: &str = "SELECT TOP (@P2) schema_name FROM information_schema.schemata \
    WHERE schema_name NOT IN ('INFORMATION_SCHEMA', 'sys', 'guest') \
      AND schema_name NOT LIKE 'db[_]%' \
      AND (@P1 = '' OR schema_name LIKE '%' + @P1 + '%' ESCAPE '\\') \
    ORDER BY schema_name";

const MSSQL_TABLES: &str = "SELECT TOP (@P3) table_name FROM information_schema.tables \
    WHERE table_schema = @P1 AND table_type = 'BASE TABLE' \
      AND (@P2 = '' OR table_name LIKE '%' + @P2 + '%' ESCAPE '\\') \
    ORDER BY table_name";

const MSSQL_COLUMNS: &str = "SELECT column_name, data_type FROM information_schema.columns \
    WHERE table_schema = @P1 AND table_name = @P2 ORDER BY ordinal_position";

const MSSQL_KEY: &str = "SELECT kcu.column_name \
    FROM information_schema.table_constraints tc \
    JOIN information_schema.key_column_usage kcu \
      ON kcu.constraint_schema = tc.constraint_schema \
     AND kcu.constraint_name = tc.constraint_name \
     AND kcu.table_name = tc.table_name \
    WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = @P1 AND tc.table_name = @P2 \
    ORDER BY kcu.ordinal_position";

/// Columns and a key as the preview shape; the types are the source's own
/// (not inferred). No column at all means the table is not there or not
/// visible to the account, said as a failure, not as an empty table.
fn declared_preview(
    columns: Vec<(String, String)>,
    key: Vec<String>,
) -> Result<SqlOut, ErrorClass> {
    if columns.is_empty() {
        return Err(ErrorClass::Failed);
    }
    Ok(SqlOut::Columns(Preview {
        columns: columns
            .into_iter()
            .map(|(name, type_name)| ListedColumn {
                name,
                type_name,
                inferred: false,
            })
            .collect(),
        primary_key: Some(key).filter(|key| !key.is_empty()),
    }))
}

fn names_out(rows: Vec<(String,)>, limit: usize) -> SqlOut {
    let (names, truncated) = bounded(rows.into_iter().map(|row| row.0).collect(), limit);
    SqlOut::Names { names, truncated }
}

async fn run_postgres(pool: &sqlx::PgPool, op: &SqlOp<'_>) -> Result<SqlOut, ErrorClass> {
    let fail = |err: sqlx::Error| logged("a PostgreSQL discovery query", class_of_sqlx(&err));
    match *op {
        SqlOp::Schemas { search, limit } => {
            let rows: Vec<(String,)> = sqlx::query_as(PG_SCHEMAS)
                .bind(like_escape(search))
                .bind(fetch_limit(limit))
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            Ok(names_out(rows, limit))
        }
        SqlOp::Tables {
            schema,
            search,
            limit,
        } => {
            let rows: Vec<(String,)> = sqlx::query_as(PG_TABLES)
                .bind(schema)
                .bind(like_escape(search))
                .bind(fetch_limit(limit))
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            Ok(names_out(rows, limit))
        }
        SqlOp::Columns { schema, table } => {
            let columns: Vec<(String, String)> = sqlx::query_as(PG_COLUMNS)
                .bind(schema)
                .bind(table)
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            let key: Vec<(String,)> = sqlx::query_as(PG_KEY)
                .bind(schema)
                .bind(table)
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            declared_preview(columns, key.into_iter().map(|row| row.0).collect())
        }
    }
}

async fn run_mysql(pool: &sqlx::MySqlPool, op: &SqlOp<'_>) -> Result<SqlOut, ErrorClass> {
    let fail = |err: sqlx::Error| logged("a MySQL discovery query", class_of_sqlx(&err));
    match *op {
        SqlOp::Schemas { search, limit } => {
            let escaped = like_escape(search);
            let rows: Vec<(String,)> = sqlx::query_as(MYSQL_SCHEMAS)
                .bind(&escaped)
                .bind(&escaped)
                .bind(fetch_limit(limit))
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            Ok(names_out(rows, limit))
        }
        SqlOp::Tables {
            schema,
            search,
            limit,
        } => {
            let escaped = like_escape(search);
            let rows: Vec<(String,)> = sqlx::query_as(MYSQL_TABLES)
                .bind(schema)
                .bind(&escaped)
                .bind(&escaped)
                .bind(fetch_limit(limit))
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            Ok(names_out(rows, limit))
        }
        SqlOp::Columns { schema, table } => {
            let columns: Vec<(String, String)> = sqlx::query_as(MYSQL_COLUMNS)
                .bind(schema)
                .bind(table)
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            let key: Vec<(String,)> = sqlx::query_as(MYSQL_KEY)
                .bind(schema)
                .bind(table)
                .fetch_all(pool)
                .await
                .map_err(fail)?;
            declared_preview(columns, key.into_iter().map(|row| row.0).collect())
        }
    }
}

type MssqlClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// First column of every row of a `tiberius` query, as text.
async fn mssql_column(
    client: &mut MssqlClient,
    query: &str,
    params: &[&dyn tiberius::ToSql],
) -> Result<Vec<Vec<String>>, ErrorClass> {
    let fail = |err: tiberius::error::Error| {
        logged("a SQL Server discovery query", class_of_tiberius(&err))
    };
    let rows = client
        .query(query, params)
        .await
        .map_err(fail)?
        .into_first_result()
        .await
        .map_err(fail)?;
    Ok(rows
        .iter()
        .map(|row| {
            (0..row.len())
                .map(|index| row.get::<&str, _>(index).unwrap_or_default().to_owned())
                .collect()
        })
        .collect())
}

async fn run_mssql(client: &mut MssqlClient, op: &SqlOp<'_>) -> Result<SqlOut, ErrorClass> {
    let first = |rows: Vec<Vec<String>>| -> Vec<(String,)> {
        rows.into_iter()
            .filter_map(|row| row.into_iter().next())
            .map(|name| (name,))
            .collect()
    };
    match *op {
        SqlOp::Schemas { search, limit } => {
            let escaped = like_escape(search);
            let rows = mssql_column(
                client,
                MSSQL_SCHEMAS,
                &[&escaped.as_str(), &fetch_limit(limit)],
            )
            .await?;
            Ok(names_out(first(rows), limit))
        }
        SqlOp::Tables {
            schema,
            search,
            limit,
        } => {
            let escaped = like_escape(search);
            let rows = mssql_column(
                client,
                MSSQL_TABLES,
                &[&schema, &escaped.as_str(), &fetch_limit(limit)],
            )
            .await?;
            Ok(names_out(first(rows), limit))
        }
        SqlOp::Columns { schema, table } => {
            let columns = mssql_column(client, MSSQL_COLUMNS, &[&schema, &table]).await?;
            let key = mssql_column(client, MSSQL_KEY, &[&schema, &table]).await?;
            declared_preview(
                columns
                    .into_iter()
                    .filter_map(|row| {
                        let mut cells = row.into_iter();
                        Some((cells.next()?, cells.next()?))
                    })
                    .collect(),
                first(key).into_iter().map(|row| row.0).collect(),
            )
        }
    }
}

/// Open a checked, bounded connection to `target` and run `op` on it. The
/// address check comes first, then the credential, then the dial to the
/// approved address (`SEC-15`), like `connector_discover::discover_dial`.
async fn run_sql(
    target: &DialTarget<'_>,
    secret_ref: &str,
    op: &SqlOp<'_>,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
) -> Result<SqlOut, ErrorClass> {
    let approved = connector_probe::resolve_checked(target.host, target.port, internal_hosts)
        .await
        .map_err(|message| class_of_address_error(&message))?;
    let password = resolver
        .resolve_dyn(secret_ref)
        .await
        .map_err(|_| logged("resolving the connector's credential", ErrorClass::Failed))?;
    let attempt = tokio::time::timeout(LIST_TIMEOUT, async {
        match target.driver {
            SqlDriver::Postgres => {
                let options = connector_probe::pg_connect_options(
                    &approved,
                    target.user,
                    password.expose_secret(),
                    target.database,
                    ca,
                );
                let pool = PgPoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await
                    .map_err(|err| logged("a PostgreSQL connection", class_of_sqlx(&err)))?;
                let out = run_postgres(&pool, op).await;
                pool.close().await;
                out
            }
            SqlDriver::Mysql => {
                let options = connector_probe::mysql_connect_options(
                    &approved,
                    target.user,
                    password.expose_secret(),
                    target.database,
                );
                let pool = MySqlPoolOptions::new()
                    .max_connections(1)
                    .connect_with(options)
                    .await
                    .map_err(|err| logged("a MySQL connection", class_of_sqlx(&err)))?;
                let out = run_mysql(&pool, op).await;
                pool.close().await;
                out
            }
            SqlDriver::Mssql => {
                let mut client = connect_mssql(&approved, target, password.expose_secret()).await?;
                run_mssql(&mut client, op).await
            }
            // Oracle is read by the orchestrator (`dispatch` never gets here).
            SqlDriver::Oracle => Err(ErrorClass::NotSupported),
        }
    })
    .await;
    attempt.unwrap_or(Err(ErrorClass::Timeout))
}

async fn connect_mssql(
    approved: &Approved,
    target: &DialTarget<'_>,
    password: &str,
) -> Result<MssqlClient, ErrorClass> {
    let mut config = tiberius::Config::new();
    config.host(target.host);
    config.port(target.port);
    config.database(target.database);
    config.authentication(tiberius::AuthMethod::sql_server(target.user, password));
    // The same stated TLS posture as `connector_probe::probe_mssql`.
    config.trust_cert();
    // The socket goes to the approved address; `config.host` keeps the name for
    // the TDS handshake (`SEC-15`).
    let tcp = connector_probe::connect_pinned(approved)
        .await
        .map_err(|_| logged("a SQL Server connection", ErrorClass::Unreachable))?;
    Box::pin(tiberius::Client::connect(config, tcp.compat_write()))
        .await
        .map_err(|err| logged("a SQL Server login", class_of_tiberius(&err)))
}

/// A SQL source's list (D7, D8, D9) or a table's columns.
async fn sql_answer(
    kind: Kind,
    target: &DialTarget<'_>,
    info: &ConnectorDialInfo,
    request: &Request,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
) -> Answer {
    let schema = request.schema.as_str();
    let search = request.search.as_str();
    let limit = request.limit;
    let op = match kind {
        Kind::List if schema.is_empty() => SqlOp::Schemas { search, limit },
        Kind::List => SqlOp::Tables {
            schema,
            search,
            limit,
        },
        Kind::Preview => {
            let Some((schema, table)) = split_object(&request.object) else {
                return Answer::failed(kind, ErrorClass::Failed);
            };
            SqlOp::Columns { schema, table }
        }
    };
    match run_sql(target, &info.secret_ref, &op, resolver, internal_hosts, ca).await {
        Ok(SqlOut::Names { names, truncated }) => {
            let (object_kind, qualifier) = if schema.is_empty() {
                (ObjectKind::Schema, "")
            } else {
                (ObjectKind::Table, schema)
            };
            let objects = names
                .into_iter()
                .map(|name| {
                    let full = if qualifier.is_empty() {
                        name
                    } else {
                        format!("{qualifier}.{name}")
                    };
                    ListedObject::named(full, object_kind)
                })
                .collect();
            Answer::listing(schema, Listing { objects, truncated })
        }
        Ok(SqlOut::Columns(preview)) => Answer::preview(&request.object, preview),
        Err(class) => Answer::failed(kind, class),
    }
}

// ── Object storage ────────────────────────────────────────────────────────

/// The S3 client for a `files` connector, built the way the connection test
/// builds it: endpoint parsed and checked (address first), credentials resolved
/// after, the dial pinned to the approved address.
async fn s3_for(
    dial: &FilesDial,
    info: &ConnectorDialInfo,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
) -> Result<AmazonS3, ErrorClass> {
    // No endpoint is AWS S3 itself, which has no address to check or pin; the
    // connection test declines it for the same reason.
    let Some(endpoint) = dial.endpoint.as_deref().filter(|e| !e.is_empty()) else {
        return Err(ErrorClass::NotSupported);
    };
    let Some((host, port)) = connector_probe::parse_endpoint_host_port(endpoint) else {
        return Err(ErrorClass::Failed);
    };
    let approved = connector_probe::resolve_checked(host, port, internal_hosts)
        .await
        .map_err(|message| class_of_address_error(&message))?;
    if !connector_probe::url_names_target(endpoint, host, port) {
        return Err(ErrorClass::Failed);
    }
    let Some(secondary) = info.secret_ref_secondary.as_deref() else {
        return Err(ErrorClass::Failed);
    };
    let unresolved = |_| logged("resolving the connector's credential", ErrorClass::Failed);
    let access_key = resolver
        .resolve_dyn(&info.secret_ref)
        .await
        .map_err(unresolved)?;
    let secret_key = resolver.resolve_dyn(secondary).await.map_err(unresolved)?;
    connector_probe::s3_client(
        endpoint,
        &dial.bucket,
        host,
        &approved,
        access_key.expose_secret(),
        secret_key.expose_secret(),
    )
    .map_err(|_| logged("building the S3 client", ErrorClass::Failed))
}

/// The files of a bucket that match a glob (D2).
async fn list_s3(
    client: &AmazonS3,
    connector_prefix: &str,
    glob: &Glob,
    search: &str,
    limit: usize,
) -> Result<Listing, ErrorClass> {
    let Some(directory) = listing_directory(connector_prefix, glob.literal_prefix()) else {
        return Ok(Listing {
            objects: Vec::new(),
            truncated: false,
        });
    };
    let prefix = (!directory.is_empty())
        .then(|| ObjectPath::parse(&directory).map_err(|_| ErrorClass::Failed))
        .transpose()?;
    let mut stream = client.list(prefix.as_ref());
    let (mut objects, mut scanned, mut truncated) = (Vec::new(), 0_usize, false);
    while let Some(item) = stream.next().await {
        let meta = item.map_err(|err| logged("an S3 listing", class_of_object_store(&err)))?;
        scanned += 1;
        if scanned > MAX_SCANNED {
            return Err(ErrorClass::TooManyObjects);
        }
        let key = meta.location.to_string();
        // A key longer than a source object's name may be is not listed: it
        // could not be picked as a table.
        if key.chars().count() > MAX_NAME_CHARS
            || !key.starts_with(connector_prefix)
            || !glob.matches(&key)
            || !contains_ignore_case(&key, search)
        {
            continue;
        }
        if objects.len() >= limit {
            truncated = true;
            break;
        }
        objects.push(ListedObject {
            name: key,
            kind: ObjectKind::File,
            size: i64::try_from(meta.size).ok(),
            modified_at: Some(
                meta.last_modified
                    .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            ),
        });
    }
    Ok(Listing { objects, truncated })
}

/// The columns of one file (D6): a CSV from its head, a Parquet from its footer.
async fn preview_s3(
    client: &AmazonS3,
    format: FilesFormat,
    key: &str,
) -> Result<Preview, ErrorClass> {
    let path = ObjectPath::parse(key).map_err(|_| ErrorClass::Failed)?;
    let fail = |err: object_store::Error| logged("an S3 read", class_of_object_store(&err));
    let size = client.head(&path).await.map_err(fail)?.size;
    match format {
        FilesFormat::Csv => {
            let end = size.min(CSV_HEAD_BYTES);
            if end == 0 {
                return Err(ErrorClass::Failed);
            }
            let head = client.get_range(&path, 0..end).await.map_err(fail)?;
            csv_preview(&head, size > end)
        }
        FilesFormat::Parquet => {
            if size == 0 || size > PARQUET_MAX_BYTES {
                return Err(ErrorClass::Failed);
            }
            let bytes = client.get_range(&path, 0..size).await.map_err(fail)?;
            let info = upload_parquet::inspect(&bytes).map_err(|_| ErrorClass::Failed)?;
            Ok(Preview {
                columns: info
                    .columns
                    .into_iter()
                    .map(|column| ListedColumn {
                        name: column.name,
                        type_name: column.type_name,
                        inferred: false,
                    })
                    .collect(),
                primary_key: None,
            })
        }
        // The loader reads CSV only (`adapters/files.py`): a preview of a format it
        // cannot load would promise a table that cannot be created.
        FilesFormat::Json => Err(ErrorClass::NotSupported),
    }
}

async fn files_answer(
    kind: Kind,
    dial: &FilesDial,
    info: &ConnectorDialInfo,
    request: &Request,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
) -> Answer {
    if dial.protocol != FilesProtocol::S3 {
        return Answer::failed(kind, ErrorClass::NotSupported);
    }
    let Some(glob) = Glob::parse(&request.glob) else {
        return Answer::failed(kind, ErrorClass::Failed);
    };
    let attempt = tokio::time::timeout(LIST_TIMEOUT, async {
        let client = s3_for(dial, info, resolver, internal_hosts).await?;
        match kind {
            Kind::List => {
                let prefix = dial.prefix.as_deref().unwrap_or_default();
                list_s3(&client, prefix, &glob, &request.search, request.limit)
                    .await
                    .map(|listing| Answer::listing(&request.glob, listing))
            }
            Kind::Preview => preview_s3(&client, dial.format, &request.object)
                .await
                .map(|preview| Answer::preview(&request.object, preview)),
        }
    })
    .await;
    match attempt {
        Ok(Ok(answer)) => answer,
        Ok(Err(class)) => Answer::failed(kind, class),
        Err(_) => Answer::failed(kind, ErrorClass::Timeout),
    }
}

// ── REST ──────────────────────────────────────────────────────────────────

/// A REST connector's list is the endpoints its dial declares (D1, D3): no
/// network, so nothing here can fail on the source.
fn rest_listing(endpoints: &[String], request: &Request) -> Listing {
    let matching: Vec<String> = endpoints
        .iter()
        .filter(|path| contains_ignore_case(path, &request.search))
        .cloned()
        .collect();
    let (names, truncated) = bounded(matching, request.limit);
    Listing {
        objects: names
            .into_iter()
            .map(|name| ListedObject::named(name, ObjectKind::Endpoint))
            .collect(),
        truncated,
    }
}

// ── The entry point ───────────────────────────────────────────────────────

/// Answer `request` from the API's own drivers, or say the orchestrator must
/// (D1). Every refusal and failure is an [`Answer`] with a class, never an
/// error: the route has one shape to return.
pub async fn dispatch(
    kind: Kind,
    info: &ConnectorDialInfo,
    request: &Request,
    resolver: &dyn DynSecretResolver,
    internal_hosts: &InternalHosts,
    ca: Option<&Path>,
) -> Dispatch {
    let Some(adapter) = info.adapter.as_deref() else {
        // No ingest spec yet: there is no dial to read.
        return Dispatch::Done(Answer::failed(kind, ErrorClass::NotSupported));
    };
    let Ok(dial) = Dial::parse(adapter, &info.dial) else {
        return Dispatch::Done(Answer::failed(kind, ErrorClass::Failed));
    };
    let answer = match &dial {
        Dial::Sql(sql) if sql.driver != SqlDriver::Oracle => {
            sql_answer(
                kind,
                &DialTarget::from(sql),
                info,
                request,
                resolver,
                internal_hosts,
                ca,
            )
            .await
        }
        Dial::Cdc(cdc) if cdc.driver != SqlDriver::Oracle => {
            sql_answer(
                kind,
                &DialTarget::from(cdc),
                info,
                request,
                resolver,
                internal_hosts,
                ca,
            )
            .await
        }
        Dial::Files(files) => {
            files_answer(kind, files, info, request, resolver, internal_hosts).await
        }
        Dial::Rest(rest) if kind == Kind::List => {
            let paths: Vec<String> = rest.endpoints.iter().map(|e| e.path.clone()).collect();
            Answer::listing("", rest_listing(&paths, request))
        }
        Dial::Sheets(_) => Answer::failed(kind, ErrorClass::NotSupported),
        // Oracle, MongoDB, Kafka, SFTP, and the column preview of REST.
        Dial::Sql(_)
        | Dial::Cdc(_)
        | Dial::Rest(_)
        | Dial::Mongo(_)
        | Dial::Kafka(_)
        | Dial::Sftp(_) => return Dispatch::Orchestrator,
    };
    Dispatch::Done(answer)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn globs(pattern: &str) -> Glob {
        Glob::parse(pattern).unwrap()
    }

    /// `SRC-13` D2: `*` stays inside a path segment, `**` crosses, `?` is one
    /// character; everything else is literal.
    #[test]
    fn globs_match_by_the_documented_rules() {
        let cases = [
            ("sales/2026-*.csv", "sales/2026-01.csv", true),
            ("sales/2026-*.csv", "sales/2026-01.txt", false),
            ("sales/*.csv", "sales/2026/01.csv", false),
            ("sales/**.csv", "sales/2026/01.csv", true),
            ("sales/**", "sales/a/b/c", true),
            ("**", "anything/at/all", true),
            ("", "anything", true),
            ("a?c", "abc", true),
            ("a?c", "a/c", false),
            ("a?c", "ac", false),
            ("a.b", "axb", false),
            ("*", "", true),
            ("a*b*c", "aXXbYYc", true),
            ("a*b*c", "aXXbYY", false),
            ("report[1].csv", "report[1].csv", true),
        ];
        for (pattern, text, expected) in cases {
            assert_eq!(
                globs(pattern).matches(text),
                expected,
                "{pattern} vs {text}"
            );
        }
    }

    /// A pattern built to make a backtracking matcher exponential finishes at
    /// once on this one, and a pattern past the bound or with a control
    /// character is refused.
    #[test]
    fn a_hostile_glob_is_linear_and_a_bad_one_is_refused() {
        let hostile = format!("{}b", "a*".repeat(60));
        let text = "a".repeat(2000);
        assert!(!globs(&hostile).matches(&text));
        assert!(Glob::parse(&"x".repeat(MAX_GLOB_CHARS + 1)).is_none());
        assert!(Glob::parse("a\nb").is_none());
    }

    #[test]
    fn the_literal_prefix_stops_at_the_first_wildcard() {
        assert_eq!(globs("sales/2026-*.csv").literal_prefix(), "sales/2026-");
        assert_eq!(globs("**").literal_prefix(), "");
        assert_eq!(globs("exact.csv").literal_prefix(), "exact.csv");
    }

    /// The bucket directory to list: the longer of the connector's prefix and
    /// the glob's literal part, cut at a `/` (`object_store` lists by whole
    /// segments); none when they disagree.
    #[test]
    fn the_listing_directory_follows_the_longer_of_the_prefix_and_the_glob() {
        assert_eq!(listing_directory("", "").as_deref(), Some(""));
        assert_eq!(listing_directory("in/", "").as_deref(), Some("in"));
        assert_eq!(
            listing_directory("in/", "in/2026/a-").as_deref(),
            Some("in/2026")
        );
        assert_eq!(listing_directory("in/2026-", "in/").as_deref(), Some("in"));
        assert_eq!(listing_directory("in/", "out/"), None);
        assert_eq!(listing_directory("", "top-").as_deref(), Some(""));
    }

    #[test]
    fn like_escape_makes_wildcards_match_themselves() {
        assert_eq!(like_escape("a_b%c[d]\\"), "a\\_b\\%c\\[d]\\\\");
        assert_eq!(like_escape("plain"), "plain");
    }

    #[test]
    fn an_object_name_splits_at_its_first_dot() {
        assert_eq!(split_object("public.orders"), Some(("public", "orders")));
        assert_eq!(split_object("a.b.c"), Some(("a", "b.c")));
        assert_eq!(split_object("orders"), None);
        assert_eq!(split_object(".orders"), None);
        assert_eq!(split_object("public."), None);
    }

    /// `SRC-13` D6: a sample's type is the narrowest one every non-blank value
    /// fits; a column with no value is a string.
    #[test]
    fn a_sample_is_typed_by_the_narrowest_type_all_values_fit() {
        let typed = |values: &[&str]| infer_type(values.iter().copied());
        assert_eq!(typed(&["1", "22", ""]), "integer");
        assert_eq!(typed(&["1", "2.5"]), "number");
        assert_eq!(typed(&["true", "FALSE"]), "boolean");
        assert_eq!(typed(&["2026-10-15", "2026-01-02"]), "date");
        assert_eq!(typed(&["2026-10-15T09:00:00Z"]), "timestamp");
        assert_eq!(typed(&["2026-10-15 09:00:00"]), "timestamp");
        assert_eq!(typed(&["1", "x"]), "string");
        assert_eq!(typed(&["", "  "]), "string");
        assert_eq!(typed(&["NaN", "inf"]), "string");
    }

    /// `SRC-13` D6: a CSV preview is the header, typed from the rows, every
    /// column marked inferred; a blank header cell is named, not dropped.
    #[test]
    fn a_csv_preview_types_each_column_from_its_rows() {
        let head = b"id,name,joined,\n1,ann,2026-01-02,x\n2,bob,2026-02-03,y\n";
        let preview = csv_preview(head, false).unwrap();
        let columns: Vec<(&str, &str, bool)> = preview
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.type_name.as_str(), c.inferred))
            .collect();
        assert_eq!(
            columns,
            [
                ("id", "integer", true),
                ("name", "string", true),
                ("joined", "date", true),
                ("column_4", "string", true),
            ]
        );
        assert!(preview.primary_key.is_none());
    }

    #[test]
    fn an_empty_csv_has_no_columns_to_preview() {
        assert_eq!(csv_preview(b"", false).unwrap_err(), ErrorClass::Failed);
    }

    /// `SRC-13` D10 (principle 4): a class comes from the error's type and
    /// code. Only the cases that can be built without a server are here; the
    /// database codes are exercised by `tests/connector_discovery_routes.rs`.
    #[test]
    fn errors_are_classified_by_type_not_text() {
        assert_eq!(
            class_of_sqlx(&sqlx::Error::PoolTimedOut),
            ErrorClass::Timeout
        );
        assert_eq!(
            class_of_sqlx(&sqlx::Error::Io(std::io::ErrorKind::TimedOut.into())),
            ErrorClass::Timeout
        );
        assert_eq!(
            class_of_sqlx(&sqlx::Error::Io(
                std::io::ErrorKind::ConnectionRefused.into()
            )),
            ErrorClass::Unreachable
        );
        assert_eq!(
            class_of_sqlx(&sqlx::Error::Tls("password=hunter2".into())),
            ErrorClass::Unreachable
        );
        assert_eq!(class_of_sqlx(&sqlx::Error::RowNotFound), ErrorClass::Failed);
        assert_eq!(
            class_of_address_error("refusing to dial \"db\": ..."),
            ErrorClass::AddressRefused
        );
        assert_eq!(
            class_of_address_error("could not resolve host \"db\""),
            ErrorClass::Unreachable
        );
    }

    /// The address check refuses a loopback host before anything is dialled or
    /// any credential is read (the resolver would panic), and says so with the
    /// class, not a message.
    #[tokio::test]
    async fn an_internal_host_is_refused_before_the_credential_is_read() {
        #[derive(Debug)]
        struct PanicIfCalled;
        #[async_trait::async_trait]
        impl DynSecretResolver for PanicIfCalled {
            async fn resolve_dyn(
                &self,
                _secret_ref: &str,
            ) -> Result<lakehouse_core::secret::SecretValue, lakehouse_core::secret::SecretError>
            {
                panic!("the address check must come before the credential");
            }
        }
        let info = ConnectorDialInfo {
            kind: "PostgreSQL".to_owned(),
            host: "x".to_owned(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some("sql".to_owned()),
            dial: serde_json::json!({
                "driver": "postgres", "host": "127.0.0.1", "port": 5432,
                "database": "d", "user": "u",
            }),
        };
        let request = Request {
            limit: 10,
            ..Request::default()
        };
        let Dispatch::Done(answer) = dispatch(
            Kind::List,
            &info,
            &request,
            &PanicIfCalled,
            &InternalHosts::NONE,
            None,
        )
        .await
        else {
            panic!("the API answers a PostgreSQL source itself");
        };
        assert_eq!(answer.error_class, Some(ErrorClass::AddressRefused));
        assert_eq!(answer.reason, Some(ErrorClass::AddressRefused.reason()));
    }

    /// `SRC-13` D1: the types the orchestrator reads are handed over, and a
    /// REST connector's list is its declared endpoints, searchable, with no
    /// network.
    #[tokio::test]
    async fn dispatch_hands_orchestrator_types_over_and_lists_rest_endpoints_itself() {
        let resolver =
            lakehouse_core::secret::EnvSecretResolver::with_map(std::collections::HashMap::new());
        let info = |adapter: &str, dial: serde_json::Value| ConnectorDialInfo {
            kind: "x".to_owned(),
            host: "x".to_owned(),
            secret_ref: "env:UNUSED".to_owned(),
            secret_ref_secondary: None,
            adapter: Some(adapter.to_owned()),
            dial,
        };
        let request = Request {
            limit: 10,
            ..Request::default()
        };
        let oracle = info(
            "sql",
            serde_json::json!({"driver": "oracle", "host": "o.example", "port": 1521,
                "database": "FREEPDB1", "user": "u"}),
        );
        let mongo = info(
            "mongodb",
            serde_json::json!({"hosts": ["m.example:27017"], "database": "d",
                "username": "u", "directConnection": true}),
        );
        for connector in [&oracle, &mongo] {
            let out = dispatch(
                Kind::List,
                connector,
                &request,
                &resolver,
                &InternalHosts::NONE,
                None,
            )
            .await;
            assert!(matches!(out, Dispatch::Orchestrator));
        }

        let rest = info(
            "rest",
            serde_json::json!({
                "baseUrl": "https://api.example", "auth": {"type": "bearer"},
                "pagination": {"type": "none"},
                "endpoints": [{"path": "/orders"}, {"path": "/customers"}, {"path": "/Order_items"}],
            }),
        );
        let search = Request {
            search: "order".to_owned(),
            ..request.clone()
        };
        let Dispatch::Done(answer) = dispatch(
            Kind::List,
            &rest,
            &search,
            &resolver,
            &InternalHosts::NONE,
            None,
        )
        .await
        else {
            panic!("the API lists REST endpoints itself");
        };
        let names: Vec<&str> = answer
            .objects
            .as_ref()
            .unwrap()
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(names, ["/orders", "/Order_items"]);
        // The column preview of a REST endpoint reads the source: the orchestrator's.
        assert!(matches!(
            dispatch(
                Kind::Preview,
                &rest,
                &request,
                &resolver,
                &InternalHosts::NONE,
                None,
            )
            .await,
            Dispatch::Orchestrator
        ));
    }

    #[tokio::test]
    async fn a_sheets_connector_and_one_without_a_spec_are_honestly_unsupported() {
        let resolver =
            lakehouse_core::secret::EnvSecretResolver::with_map(std::collections::HashMap::new());
        let request = Request {
            limit: 10,
            ..Request::default()
        };
        let connector = |adapter: Option<&str>, dial: serde_json::Value| ConnectorDialInfo {
            kind: "x".to_owned(),
            host: "x".to_owned(),
            secret_ref: String::new(),
            secret_ref_secondary: None,
            adapter: adapter.map(str::to_owned),
            dial,
        };
        for info in [
            connector(None, serde_json::json!({})),
            connector(
                Some("sheets"),
                serde_json::json!({"spreadsheetId": "abc", "ranges": ["A1:B2"]}),
            ),
        ] {
            let Dispatch::Done(answer) = dispatch(
                Kind::List,
                &info,
                &request,
                &resolver,
                &InternalHosts::NONE,
                None,
            )
            .await
            else {
                panic!("answered by the API");
            };
            assert_eq!(answer.error_class, Some(ErrorClass::NotSupported));
            assert!(
                !answer.reason.unwrap().contains("unknown adapter"),
                "the wrong text of before SRC-13 is gone"
            );
        }
    }

    // ── live PostgreSQL ────────────────────────────────────────────────────

    #[sqlx::test(migrations = "../../migrations")]
    async fn postgres_schemas_hide_the_system_ones_and_honour_search_and_limit(pool: sqlx::PgPool) {
        sqlx::query("CREATE SCHEMA sales_eu")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE SCHEMA sales_us")
            .execute(&pool)
            .await
            .unwrap();
        let all = run_postgres(
            &pool,
            &SqlOp::Schemas {
                search: "",
                limit: 100,
            },
        )
        .await
        .unwrap();
        let SqlOut::Names { names, truncated } = all else {
            panic!("names");
        };
        assert!(!truncated);
        assert!(names.contains(&"public".to_owned()));
        assert!(names.contains(&"sales_eu".to_owned()));
        assert!(
            !names
                .iter()
                .any(|n| n == "information_schema" || n == "pg_catalog" || n.starts_with("pg_")),
            "{names:?}"
        );

        let found = run_postgres(
            &pool,
            &SqlOp::Schemas {
                search: "SALES_E",
                limit: 100,
            },
        )
        .await
        .unwrap();
        let SqlOut::Names { names, .. } = found else {
            panic!("names");
        };
        assert_eq!(names, ["sales_eu"]);

        let cut = run_postgres(
            &pool,
            &SqlOp::Schemas {
                search: "sales",
                limit: 1,
            },
        )
        .await
        .unwrap();
        let SqlOut::Names { names, truncated } = cut else {
            panic!("names");
        };
        assert_eq!(names, ["sales_eu"]);
        assert!(truncated, "one more existed than the limit");
    }

    /// `SRC-13` D8, D9: tables only (not views), names first; a search treats
    /// `_` and `%` as themselves; the limit cuts and says so.
    #[sqlx::test(migrations = "../../migrations")]
    async fn postgres_tables_are_base_tables_only_and_the_search_is_literal(pool: sqlx::PgPool) {
        // SRC-13 review 2 BLOCKER 1: `public` of a `sqlx::test` database also holds
        // every migration's tables (`_sqlx_migrations`, `access_grant`, ...), so the
        // exact-list assertions below ran against more than their fixtures. The test
        // lists a schema of its own, which holds exactly the four fixtures.
        const SCHEMA: &str = "src13_tables_fixture";
        for ddl in [
            "CREATE SCHEMA src13_tables_fixture",
            "CREATE TABLE src13_tables_fixture.a_b (id INT)",
            "CREATE TABLE src13_tables_fixture.axb (id INT)",
            "CREATE TABLE src13_tables_fixture.c100 (id INT)",
            "CREATE VIEW src13_tables_fixture.a_view AS SELECT 1 AS id",
        ] {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }
        let list = |search: &'static str, limit: usize| {
            let pool = pool.clone();
            async move {
                match run_postgres(
                    &pool,
                    &SqlOp::Tables {
                        schema: SCHEMA,
                        search,
                        limit,
                    },
                )
                .await
                .unwrap()
                {
                    SqlOut::Names { names, truncated } => (names, truncated),
                    SqlOut::Columns(_) => panic!("names"),
                }
            }
        };
        let (names, truncated) = list("", 100).await;
        assert_eq!(names, ["a_b", "axb", "c100"], "the view is not listed");
        assert!(!truncated);
        let (names, _) = list("a_b", 100).await;
        assert_eq!(names, ["a_b"], "`_` is not a wildcard");
        let (names, _) = list("%", 100).await;
        assert!(names.is_empty(), "`%` is not a wildcard");
        let (names, truncated) = list("", 2).await;
        assert_eq!(names, ["a_b", "axb"]);
        assert!(truncated);
        sqlx::query("DROP SCHEMA src13_tables_fixture CASCADE")
            .execute(&pool)
            .await
            .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn postgres_columns_come_with_types_and_the_key_in_key_order(pool: sqlx::PgPool) {
        sqlx::query(
            "CREATE TABLE keyed (region TEXT, id INT, note TEXT, PRIMARY KEY (id, region))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("CREATE TABLE unkeyed (id INT)")
            .execute(&pool)
            .await
            .unwrap();
        let SqlOut::Columns(keyed) = run_postgres(
            &pool,
            &SqlOp::Columns {
                schema: "public",
                table: "keyed",
            },
        )
        .await
        .unwrap() else {
            panic!("columns");
        };
        let columns: Vec<(&str, &str, bool)> = keyed
            .columns
            .iter()
            .map(|c| (c.name.as_str(), c.type_name.as_str(), c.inferred))
            .collect();
        assert_eq!(
            columns,
            [
                ("region", "text", false),
                ("id", "integer", false),
                ("note", "text", false)
            ]
        );
        assert_eq!(keyed.primary_key.unwrap(), ["id", "region"]);

        let SqlOut::Columns(unkeyed) = run_postgres(
            &pool,
            &SqlOp::Columns {
                schema: "public",
                table: "unkeyed",
            },
        )
        .await
        .unwrap() else {
            panic!("columns");
        };
        assert!(unkeyed.primary_key.is_none(), "no key is not an empty key");

        let missing = run_postgres(
            &pool,
            &SqlOp::Columns {
                schema: "public",
                table: "nope",
            },
        )
        .await
        .unwrap_err();
        assert_eq!(missing, ErrorClass::Failed);
    }

    /// The query texts bind every value, once each, in the driver's own
    /// placeholder syntax, and never interpolate one.
    #[test]
    fn the_listing_queries_bind_their_values_and_interpolate_none() {
        for (query, placeholder, count) in [
            (PG_SCHEMAS, "$", 3),
            (PG_TABLES, "$", 4),
            (PG_COLUMNS, "$", 2),
            (PG_KEY, "$", 2),
            (MYSQL_SCHEMAS, "?", 3),
            (MYSQL_TABLES, "?", 4),
            (MYSQL_COLUMNS, "?", 2),
            (MYSQL_KEY, "?", 2),
            (MSSQL_SCHEMAS, "@P", 3),
            (MSSQL_TABLES, "@P", 4),
            (MSSQL_COLUMNS, "@P", 2),
            (MSSQL_KEY, "@P", 2),
        ] {
            assert_eq!(query.matches(placeholder).count(), count, "{query}");
            assert!(!query.contains('{') && !query.contains('}'), "{query}");
        }
        for query in [
            MSSQL_TABLES,
            MSSQL_SCHEMAS,
            MYSQL_TABLES,
            MYSQL_SCHEMAS,
            PG_TABLES,
            PG_SCHEMAS,
        ] {
            assert!(
                query.contains("ORDER BY"),
                "a cut list must be the same list every time: {query}"
            );
        }
        for query in [PG_TABLES, MYSQL_TABLES, MSSQL_TABLES] {
            assert!(query.contains("'BASE TABLE'"), "views are not listed (D8)");
        }
    }
}
