//! Per-adapter `dial` validation for registry-driven ingestion
//! (`docs/adr/0013-registry-driven-ingestion.md`).
//!
//! `connector.dial` (`0033_connector_ingest_spec.sql`) is free-form
//! `JSONB` at the column level, but never free-form at the application
//! level: [`Dial::parse`] parses it into exactly one of five
//! `#[serde(deny_unknown_fields, rename_all = "camelCase")]` structs,
//! dispatched on the connector's own `adapter` column value. Every one of
//! the five structs rejects an unknown field, which is the property that
//! stops a credential (a `password`, an `apiSecret`) being smuggled into
//! `dial` — a column the allowlisted `secretRef` resolver
//! (`lakehouse_core::secret`, ADR 0002) never inspects.
//!
//! # Never `#[serde(untagged)]`
//!
//! [`Dial::parse`] dispatches on the stored `adapter` column value and
//! then parses against exactly one struct. An untagged enum would instead
//! try every variant in turn: a malformed `dial` for a known adapter could
//! silently match a structurally-similar but wrong shape instead of
//! failing with that adapter's own named field-path error. See ADR 0013's
//! "Never `#[serde(untagged)]`" consequence for the full argument.

use serde::Deserialize;

/// A closed set of SQL/CDC drivers `dial` may name — never a free-form
/// string, so a typo (`"postgress"`) fails at parse time rather than at
/// the point some later code tries to dispatch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlDriver {
    /// `MySQL`/`MariaDB`.
    Mysql,
    /// `PostgreSQL`.
    Postgres,
    /// Microsoft SQL Server, dialed via ODBC by Task F2's Python adapter.
    Mssql,
}

/// The connection shape shared by a batch `sql` adapter's `dial`.
///
/// `user` is a literal, non-`secretRef`-shaped string — never
/// `"env:CONNECTOR_<ID>_USER"` or any other indirection scheme. This is a
/// deliberate correction against an earlier draft that put `user` behind
/// the same `secretRef`-style indirection as a password: a username is
/// not a credential, and the seeded `conn-pg-lakehouse` row already
/// encodes its username directly in `host`
/// (`lakehouse@postgres:5432/lakehouse`,
/// `0022_prune_connector_seed.sql`) rather than through any resolver — see
/// `docs/adr/0013-registry-driven-ingestion.md`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SqlDial {
    /// The SQL driver to dial.
    pub driver: SqlDriver,
    /// The hostname or IP literal to connect to, validated by
    /// [`validate_hostname`] before this `dial` is accepted.
    pub host: String,
    /// The TCP port to connect to.
    pub port: u16,
    /// The database (or schema, for engines without a separate concept)
    /// name.
    pub database: String,
    /// A literal username, never a `secretRef`-shaped indirection — see
    /// this module's own doc comment and [`SqlDial`]'s doc comment.
    pub user: String,
    /// An optional TLS mode string, passed through verbatim to the
    /// generated ingestion job.
    pub ssl_mode: Option<String>,
}

/// The connection shape for a change-data-capture `cdc` adapter's `dial`.
///
/// `slot_name`/`publication_name` are **required**, not `Option`: a `cdc`
/// connector's replication slot/publication come from `dial` with no
/// fallback, so a `cdc` dial missing either is a construction-time (400)
/// error, not a later runtime surprise — see
/// `docs/adr/0013-registry-driven-ingestion.md`'s "the same connection
/// fields as `sql`, plus `slotName` and `publicationName` as **required**"
/// decision.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CdcDial {
    /// The SQL driver to dial.
    pub driver: SqlDriver,
    /// The hostname or IP literal to connect to, validated by
    /// [`validate_hostname`] before this `dial` is accepted.
    pub host: String,
    /// The TCP port to connect to.
    pub port: u16,
    /// The database (or schema, for engines without a separate concept)
    /// name.
    pub database: String,
    /// A literal username, never a `secretRef`-shaped indirection — see
    /// [`SqlDial::user`].
    pub user: String,
    /// The replication slot name, required with no fallback.
    pub slot_name: String,
    /// The publication name, required with no fallback.
    pub publication_name: String,
    /// An optional numeric server id, used by drivers (e.g. `MySQL`
    /// binlog replication) that need one.
    pub server_id: Option<u32>,
}

/// A closed set of object-storage protocols a `files` adapter's `dial`
/// may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesProtocol {
    /// S3-compatible object storage (including `RustFS`).
    S3,
    /// SFTP.
    Sftp,
}

/// A closed set of file formats a `files` adapter's `dial` may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesFormat {
    /// Comma-separated values.
    Csv,
    /// Newline-delimited JSON.
    Json,
    /// Apache Parquet.
    Parquet,
}

/// The connection shape for a `files` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FilesDial {
    /// The object-storage protocol to dial.
    pub protocol: FilesProtocol,
    /// An optional endpoint override (e.g. a `RustFS`/`MinIO` endpoint
    /// URL rather than public AWS S3).
    pub endpoint: Option<String>,
    /// The bucket (or SFTP root) name.
    pub bucket: String,
    /// An optional key/path prefix to scope ingestion to.
    pub prefix: Option<String>,
    /// The file format to parse.
    pub format: FilesFormat,
    /// An optional region string, passed through verbatim.
    pub region: Option<String>,
}

/// One `rest` adapter authentication shape, internally tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum RestAuth {
    /// A named header carries the API key. The key's actual value is
    /// still only ever named by the connector's `secretRef`, never
    /// present in this struct.
    #[serde(rename = "api_key")]
    ApiKey {
        /// The header name the API key is sent under.
        header: String,
    },
    /// A bearer token carried in the `Authorization` header.
    Bearer,
    /// `OAuth2` client-credentials grant.
    #[serde(rename = "oauth2_client_credentials")]
    Oauth2ClientCredentials {
        /// The token endpoint URL to exchange credentials at.
        token_url: String,
    },
    /// HTTP Basic authentication.
    Basic,
}

/// A `rest` adapter's pagination shape, internally tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum RestPagination {
    /// No pagination — a single request returns the full result set.
    None,
    /// Page-number-based pagination.
    Page {
        /// The query parameter name carrying the page number.
        param: String,
    },
    /// Cursor-based pagination.
    Cursor {
        /// The JSON path in a response body naming the next cursor.
        cursor_field: String,
    },
}

/// One endpoint a `rest` adapter's `dial` enumerates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RestEndpoint {
    /// The path, relative to `RestDial::base_url`.
    pub path: String,
    /// The JSON path in a response body naming the record array, when the
    /// endpoint does not return a bare array at its root.
    pub records_path: Option<String>,
}

/// The connection shape for a `rest` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RestDial {
    /// The base URL every [`RestEndpoint::path`] is relative to.
    pub base_url: String,
    /// The authentication shape to use.
    pub auth: RestAuth,
    /// The pagination shape to use.
    pub pagination: RestPagination,
    /// The endpoints to poll.
    pub endpoints: Vec<RestEndpoint>,
}

/// The connection shape for a `sheets` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SheetsDial {
    /// The Google Sheets spreadsheet id.
    pub spreadsheet_id: String,
    /// The A1-notation ranges to read.
    pub ranges: Vec<String>,
}

/// A parsed `dial`, one variant per adapter.
///
/// Never `#[serde(untagged)]` — see this module's doc comment. Constructed
/// only via [`Dial::parse`], which dispatches on the connector's own
/// `adapter` column value rather than deriving the shape from the JSON
/// itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Dial {
    /// A batch `sql` adapter's dial.
    Sql(SqlDial),
    /// A change-data-capture `cdc` adapter's dial.
    Cdc(CdcDial),
    /// A `files` adapter's dial.
    Files(FilesDial),
    /// A `rest` adapter's dial.
    Rest(RestDial),
    /// A `sheets` adapter's dial.
    Sheets(SheetsDial),
}

impl Dial {
    /// Parse `raw` into the [`Dial`] shape named by `adapter`.
    ///
    /// Dispatches on `adapter` — the connector's own stored column value —
    /// and parses `raw` against exactly one struct, never trying every
    /// shape in turn. `host` is additionally checked by
    /// [`validate_hostname`] for `sql`/`cdc` adapters, since Task F2's
    /// Python-side ODBC adapter later interpolates `host` into a
    /// delimiter-separated connection string.
    ///
    /// # Errors
    ///
    /// Returns [`IngestSpecError::UnknownAdapter`] if `adapter` is not one
    /// of `sql`/`cdc`/`files`/`rest`/`sheets`, or
    /// [`IngestSpecError::InvalidDial`] if `raw` does not match the named
    /// adapter's shape (an unknown field, a missing required field, or a
    /// hostname [`validate_hostname`] rejects).
    pub fn parse(adapter: &str, raw: &serde_json::Value) -> Result<Dial, IngestSpecError> {
        match adapter {
            "sql" => {
                let dial: SqlDial = parse_dial(adapter, raw)?;
                check_hostname(adapter, "host", &dial.host)?;
                Ok(Dial::Sql(dial))
            }
            "cdc" => {
                let dial: CdcDial = parse_dial(adapter, raw)?;
                check_hostname(adapter, "host", &dial.host)?;
                Ok(Dial::Cdc(dial))
            }
            "files" => Ok(Dial::Files(parse_dial(adapter, raw)?)),
            "rest" => Ok(Dial::Rest(parse_dial(adapter, raw)?)),
            "sheets" => Ok(Dial::Sheets(parse_dial(adapter, raw)?)),
            other => Err(IngestSpecError::UnknownAdapter {
                adapter: other.to_owned(),
            }),
        }
    }

    /// A typed accessor for the `cdc` variant, so a caller that only cares
    /// about the `cdc` shape (e.g. slot/publication deprovisioning) never
    /// needs its own `match` on the variant name.
    #[must_use]
    pub fn as_cdc(&self) -> Option<&CdcDial> {
        match self {
            Dial::Cdc(dial) => Some(dial),
            _ => None,
        }
    }
}

/// Deserialize `raw` into `T`, wrapping a failure as
/// [`IngestSpecError::InvalidDial`] named after `adapter`.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if `raw` does not match `T`'s
/// shape.
fn parse_dial<T: for<'de> Deserialize<'de>>(
    adapter: &str,
    raw: &serde_json::Value,
) -> Result<T, IngestSpecError> {
    serde_json::from_value(raw.clone()).map_err(|source| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source,
    })
}

/// Check `host` via [`validate_hostname`], wrapping a rejection as
/// [`IngestSpecError::InvalidDial`] under the same variant a
/// `deny_unknown_fields` rejection already produces, so callers handle one
/// error shape.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if `host` is not a valid
/// hostname per [`validate_hostname`].
fn check_hostname(adapter: &str, field: &'static str, host: &str) -> Result<(), IngestSpecError> {
    validate_hostname(field, host).map_err(|err| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source: serde::de::Error::custom(err),
    })
}

/// A `dial` failed to parse or validate.
///
/// [`IngestSpecError::InvalidDial`] never echoes the raw `dial` JSON —
/// only serde's own field-path message (e.g. "unknown field `password`,
/// expected one of ...") — since `dial` is exactly where a smuggled
/// credential would be (AGENTS.md principle 4, applied to input rather
/// than upstream `sqlx` errors: never surface unclassified raw text).
#[derive(Debug, thiserror::Error)]
pub enum IngestSpecError {
    /// The connector's `adapter` column names a value not in the closed
    /// `sql | cdc | files | rest | sheets` set.
    #[error("unknown adapter {adapter:?}")]
    UnknownAdapter {
        /// The offending adapter value.
        adapter: String,
    },
    /// `dial` failed to deserialize into the shape `adapter` names, or
    /// failed [`validate_hostname`]. Carries only `source`'s own message
    /// (serde's field-path text, or [`InvalidHostname`]'s message via
    /// `serde::de::Error::custom`) — never the raw JSON passed to
    /// [`Dial::parse`].
    #[error("dial for adapter {adapter:?} is invalid: {source}")]
    InvalidDial {
        /// The adapter whose shape `dial` failed to match.
        adapter: String,
        /// The underlying serde error. Its `Display` never echoes the raw
        /// input value, only the field path and expectation.
        source: serde_json::Error,
    },
}

/// One object (table, endpoint, sheet range) an ingest job targets.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceObject {
    /// `"<schema>.<table>"`-shaped for `sql`/`cdc`, or an adapter-specific
    /// name otherwise.
    pub name: String,
    /// An optional column used for incremental (watermark-based) reads.
    pub incremental_key: Option<String>,
    /// The Bronze target this object lands at.
    pub target: String,
}

/// This [`SourceObject`] failed application-level validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("source object {name:?} has an empty target")]
pub struct InvalidSourceObject {
    /// The offending source object's name.
    pub name: String,
}

impl SourceObject {
    /// Reject a [`SourceObject`] with an empty `target`: an ingest job
    /// with nowhere to land its output is a construction-time error, not
    /// a later runtime surprise.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSourceObject`] if `target` is empty.
    pub fn validate(&self) -> Result<(), InvalidSourceObject> {
        if self.target.is_empty() {
            Err(InvalidSourceObject {
                name: self.name.clone(),
            })
        } else {
            Ok(())
        }
    }
}

/// `host`/`value` is not a DNS hostname or IP-literal shape safe to
/// interpolate into a delimiter-separated text format.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field} is not a valid hostname: {value:?}")]
pub struct InvalidHostname {
    /// The field name, for error messages only.
    pub field: &'static str,
    /// The rejected value.
    pub value: String,
}

/// A DNS hostname or IP-literal shape safe to interpolate into a
/// delimiter-separated text format (an ODBC connection string, a DSN)
/// without further quoting: RFC 1123 labels (letters/digits/hyphens, no
/// leading/trailing hyphen, 1-63 chars each) separated by `.`, total
/// length <= 253, and — the property THIS rule exists for — no `;`, no
/// `{`/`}`, no control character, since those are exactly the characters
/// an ODBC/DSN-shaped consumer treats specially.
///
/// This is the one canonical hostname rule this workstream cites, so that
/// no interpolation site has to invent its own: `SqlDial`/`CdcDial`'s
/// `host` is checked here, at `dial`-construction time, in Rust; the same
/// rule is ported verbatim to Python by the Dagster `sql` adapter's own
/// hostname check (`dagster/dispar_orchestrate/adapters/sql.py`), which
/// interpolates `host` into an ODBC connection string for `mssql`.
///
/// # Errors
///
/// Returns [`InvalidHostname`] if `host` does not match the shape
/// described above.
pub fn validate_hostname(field: &'static str, host: &str) -> Result<(), InvalidHostname> {
    let labels: Vec<&str> = host.split('.').collect();
    let ok = !host.is_empty()
        && host.len() <= 253
        && !labels.is_empty()
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if ok {
        Ok(())
    } else {
        Err(InvalidHostname {
            field,
            value: host.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn sql_dial_parses_the_documented_shape() {
        let json = serde_json::json!({
            "driver": "mysql", "host": "db.internal", "port": 3306,
            "database": "orders", "user": "app_reader", "sslMode": "required"
        });
        let dial: SqlDial = serde_json::from_value(json).expect("valid sql dial");
        assert_eq!(dial.driver, SqlDriver::Mysql);
        assert_eq!(dial.user, "app_reader");
    }

    #[test]
    fn sql_dial_rejects_a_password_field() {
        let json = serde_json::json!({
            "driver": "mysql", "host": "db.internal", "port": 3306,
            "database": "orders", "user": "app_reader", "password": "s3cret"
        });
        let err = serde_json::from_value::<SqlDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn cdc_dial_requires_slot_and_publication_name() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader"
        });
        let err = serde_json::from_value::<CdcDial>(json).unwrap_err();
        assert!(err.to_string().contains("missing field"));
    }

    #[test]
    fn cdc_dial_rejects_a_password_field() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub",
            "password": "s3cret"
        });
        let err = serde_json::from_value::<CdcDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn files_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "protocol": "s3", "bucket": "b", "format": "csv", "accessKey": "AKIA..."
        });
        let err = serde_json::from_value::<FilesDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn rest_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "baseUrl": "https://api.example.com",
            "auth": {"type": "bearer"},
            "pagination": {"type": "none"},
            "endpoints": [],
            "clientSecret": "s3cret"
        });
        let err = serde_json::from_value::<RestDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn sheets_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "spreadsheetId": "abc123", "ranges": ["Sheet1!A1:B2"], "apiKey": "s3cret"
        });
        let err = serde_json::from_value::<SheetsDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn dial_parse_dispatches_on_the_adapter_column_value() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader"
        });
        let dial = Dial::parse("sql", &json).expect("valid sql dial");
        assert!(matches!(dial, Dial::Sql(_)));
        assert!(dial.as_cdc().is_none());
    }

    #[test]
    fn dial_parse_rejects_an_unknown_adapter() {
        let err = Dial::parse("smtp", &serde_json::json!({})).unwrap_err();
        assert!(matches!(err, IngestSpecError::UnknownAdapter { adapter } if adapter == "smtp"));
    }

    #[test]
    fn dial_parse_never_echoes_the_raw_dial_json() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader", "password": "s3cret-value"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        let message = err.to_string();
        assert!(!message.contains("s3cret-value"));
        assert!(message.contains("unknown field"));
    }

    #[test]
    fn dial_parse_rejects_a_host_with_a_semicolon_via_invalid_dial() {
        let json = serde_json::json!({
            "driver": "mssql", "host": "db.internal;TrustServerCertificate=yes", "port": 1433,
            "database": "orders", "user": "app_reader"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        assert!(matches!(err, IngestSpecError::InvalidDial { adapter, .. } if adapter == "sql"));
    }

    #[test]
    fn as_cdc_returns_the_cdc_dial_for_a_cdc_adapter() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub"
        });
        let dial = Dial::parse("cdc", &json).expect("valid cdc dial");
        assert_eq!(
            dial.as_cdc().map(|cdc| cdc.slot_name.as_str()),
            Some("orders_slot")
        );
    }

    #[test]
    fn source_object_validate_rejects_an_empty_target() {
        let object = SourceObject {
            name: "public.orders".to_owned(),
            incremental_key: None,
            target: String::new(),
        };
        let err = object.validate().unwrap_err();
        assert_eq!(err.name, "public.orders");
    }

    #[test]
    fn source_object_validate_accepts_a_non_empty_target() {
        let object = SourceObject {
            name: "public.orders".to_owned(),
            incremental_key: Some("updated_at".to_owned()),
            target: "bronze.orders".to_owned(),
        };
        assert!(object.validate().is_ok());
    }

    #[test]
    fn validate_hostname_rejects_a_semicolon_and_braces() {
        assert!(validate_hostname("host", "db.internal;TrustServerCertificate=yes").is_err());
        assert!(validate_hostname("host", "db{internal}").is_err());
        assert!(validate_hostname("host", "db.internal").is_ok());
    }

    #[test]
    fn validate_hostname_rejects_a_leading_hyphen_label() {
        assert!(validate_hostname("host", "-bad.internal").is_err());
    }

    #[test]
    fn validate_hostname_rejects_an_empty_host() {
        assert!(validate_hostname("host", "").is_err());
    }
}
