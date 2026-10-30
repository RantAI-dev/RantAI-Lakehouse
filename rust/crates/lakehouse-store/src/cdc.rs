//! Connector registry -> `Kafka Connect` connector config (ADR 0007, ADR 0016),
//! and the R7 schema-evolution gate ADR 0006 named but deferred to this phase:
//! nested struct/array/map source columns are rejected here, at connector
//! registration, not discovered later at read time as an unreadable Bronze
//! column.
//!
//! `SRC-4` slice 2 (decision S2-h) removed the `Debezium Server` properties
//! renderer this module used to hold; [`render_connect_config`] is the one
//! renderer now.
//!
//! # What this module does NOT do
//!
//! It does not call out to a source database to discover its schema, does
//! not open a network connection anywhere, and does not persist anything.
//! [`reject_unsupported_column_types`] is a pure function over a
//! caller-supplied column list. [`render_connect_config`] is a pure function
//! over already-resolved inputs; the password is never an input, only a
//! reference to a file the API writes (ADR 0016 point 4). Actually creating the
//! connector on `Kafka Connect` is the API's job (`kafka_connect`,
//! `cdc_lifecycle`), not this crate's.

use std::collections::BTreeMap;

use crate::ingest_spec::{CdcDial, CdcMongoDial, CdcSslMode, SqlDriver};

/// One column of a source table, as a future schema-discovery step would
/// report it — `type_name` is whatever string the source system names the
/// column's type (e.g. `Postgres`'s `information_schema.columns.data_type`,
/// or `Debezium`'s own Kafka Connect schema type name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColumn {
    /// Column name, for error messages only.
    pub name: String,
    /// The source system's own type name, case-insensitive.
    pub type_name: String,
}

/// A column whose type this connector contract cannot propagate to Bronze.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "column {column:?} has type {type_name:?}, which ClickHouse cannot read as Iceberg \
         nested data (R7): reject this connector at registration, not after Bronze already \
         has unreadable data"
)]
pub struct UnsupportedColumnType {
    /// The offending column's name.
    pub column: String,
    /// The offending column's source type name, as given.
    pub type_name: String,
}

/// Type-name fragments that mark a nested struct/array/map shape,
/// independent of source system naming convention: Postgres composite
/// types and Avro/Kafka-Connect-style nested schemas all surface one of
/// these words (or, for arrays, a trailing `[]` Postgres uses for its
/// array-of-`T` display form). Deliberately over-inclusive (a false
/// positive just means a legitimate scalar type needs a naming exception
/// added here) rather than under-inclusive, matching
/// `connectors::looks_like_raw_secret`'s "loose is the safe direction" call
/// for this exact kind of defense-in-depth heuristic.
const NESTED_TYPE_MARKERS: [&str; 6] = ["struct", "record", "array", "map", "composite", "row"];

/// Reject a source schema containing a nested struct/array/map column,
/// **before** a connector is allowed to start writing to Bronze — R7's
/// mitigation, implemented at the boundary ADR 0006 named for it
/// (registration-time, source-side).
///
/// Scalar types — including `json`/`jsonb`, which `Debezium`/dlt both
/// represent as a `String` column, not a nested Iceberg type — are always
/// allowed; only genuinely nested shapes are rejected.
///
/// # Errors
///
/// Returns the first [`UnsupportedColumnType`] found, column order as
/// given. Does not aggregate every offending column — one rejection is
/// enough to fail registration, and the caller can re-submit after fixing
/// it.
pub fn reject_unsupported_column_types(
    columns: &[SourceColumn],
) -> Result<(), UnsupportedColumnType> {
    for column in columns {
        let lower = column.type_name.to_lowercase();
        let is_array_display = lower.trim_end().ends_with("[]");
        let has_marker = NESTED_TYPE_MARKERS.iter().any(|m| lower.contains(m));
        if is_array_display || has_marker {
            return Err(UnsupportedColumnType {
                column: column.name.clone(),
                type_name: column.type_name.clone(),
            });
        }
    }
    Ok(())
}

/// Everything that can go wrong validating a slug, a connector id or the
/// inputs of [`render_connect_config`] — always an input problem, never something discovered by talking to
/// a real source database (this module never opens a connection).
///
/// Deliberately carries no credential value in any variant: the whole point
/// of validating BEFORE rendering is that a bad value never has to be
/// echoed back to explain what was wrong with it. See
/// [`ControlCharacterInField`](CdcSpecError::ControlCharacterInField)'s doc
/// comment.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CdcSpecError {
    /// A connector slug did not match [`ConnectorSlug`]'s allowed shape.
    /// The offending slug itself is safe to include here — a slug is never
    /// a credential — which is what makes this diagnosable in a way a bad
    /// password never can be.
    #[error(
        "connector slug {slug:?} is invalid: must match ^[a-z0-9][a-z0-9_]{{0,62}}$ (see \
         ConnectorSlug's doc comment for why this exact shape)"
    )]
    InvalidConnectorSlug {
        /// The rejected slug, as given.
        slug: String,
    },

    /// One of the values that gets interpolated into a connector config
    /// contained a control character (most importantly `\n`/`\r`, but any
    /// other ASCII control character is rejected too, for the same reason).
    ///
    /// Deliberately does NOT carry the offending value: some of the fields
    /// this guards (`database_password`, `s3_access_key`, `s3_secret_key`,
    /// the `Lakekeeper` bearer token) are credentials, and an error that
    /// echoed "here is the bad value" back to a log or an API response
    /// would leak exactly the secret this whole module exists to keep out
    /// of error messages. The field NAME is enough to fix the problem.
    #[error(
        "field {field} contains a control character (a newline, most dangerously): a Debezium \
         `.properties` file has no quoting mechanism, so a raw newline in a value does not get \
         escaped, it terminates that line and starts injecting whatever additional properties \
         follow it — see reject_control_characters's doc comment"
    )]
    ControlCharacterInField {
        /// Which field was rejected (never the value itself — see above).
        field: &'static str,
    },

    /// A connector id that cannot be a Kafka Connect connector name, a topic
    /// prefix and a file name at once (`SRC-4` T2.1). The id is a registry
    /// key, never a credential, so it is safe to carry.
    #[error("connector id {id:?} is invalid: must match ^[A-Za-z0-9][A-Za-z0-9_-]{{0,62}}$")]
    InvalidConnectorId {
        /// The rejected id, as given.
        id: String,
    },

    /// A source table that is not `schema.table` of plain identifier
    /// characters. `table.include.list` is a comma-separated list of regular
    /// expressions, so a comma or a metacharacter in a name would change what
    /// is captured instead of naming a table.
    #[error("source table {name:?} is not a schema.table of letters, digits, '_' and '$'")]
    InvalidTableName {
        /// The rejected name, as given (a table name is not a credential).
        name: String,
    },

    /// The secrets directory cannot be named in a config-provider reference.
    #[error("the change-capture secrets directory is not an absolute path of plain characters")]
    InvalidSecretsDir,

    /// No source table to capture.
    #[error("a change-capture connector needs at least one source table")]
    NoTables,

    /// A slot or publication name `PostgreSQL` logical replication cannot
    /// use (`Debezium` takes lowercase letters, digits and `_` only).
    #[error("{field} must be 1 to 63 lowercase letters, digits or underscores")]
    InvalidReplicationName {
        /// Which of the two names it was.
        field: &'static str,
    },

    /// The dial names a driver this renderer has no `Debezium` connector
    /// for yet (`Oracle` comes with its slice).
    #[error("change capture is not built for this source driver yet")]
    UnsupportedDriver,

    /// A `MySQL` dial without its `serverId` (`Dial::parse` requires it; a
    /// hand-built dial is refused here the same way).
    #[error("a MySQL change-capture connector needs a serverId")]
    NoServerId,

    /// `MySQL` and `SQL Server` connectors keep their schema history in a Kafka
    /// topic and cannot start without the broker's address.
    #[error("a MySQL or SQL Server change-capture connector needs the schema-history broker")]
    NoSchemaHistoryBroker,

    /// The schema-history broker list is not `host:port[,host:port]` of plain
    /// characters.
    #[error("the schema-history broker is not a host:port list of plain characters")]
    InvalidSchemaHistoryBroker,

    /// A `MongoDB` dial whose `hosts` are not `host:port` of plain characters
    /// (`Dial::parse` checks them; a hand-built dial is refused here the same
    /// way). The seeds are written into the connection string, so a `/`, `?` or
    /// `,` inside one would change what the worker dials.
    #[error(
        "a MongoDB change-capture connector's hosts are not host:port entries of plain characters"
    )]
    InvalidMongoHosts,

    /// A `MongoDB` replica-set name outside letters, digits, `_`, `-` and `.`.
    #[error("a MongoDB replica set name is 1 to 128 letters, digits, '_', '-' and '.'")]
    InvalidReplicaSet,

    /// A `MongoDB` database name outside letters, digits, `_` and `-`.
    #[error("a MongoDB database name is 1 to 63 letters, digits, '_' and '-'")]
    InvalidMongoDatabase,

    /// A `MongoDB` collection that is not one plain name. `collection.include.list`
    /// is a list of regular expressions and the collection becomes part of a topic
    /// name, so a dot or a metacharacter would change what is captured.
    #[error("source collection {name:?} is not a plain name of letters, digits, '_' and '-'")]
    InvalidCollectionName {
        /// The rejected name, as given (a collection name is not a credential).
        name: String,
    },
}

/// A connector identifier constrained to `^[a-z0-9][a-z0-9_]{0,62}$` —
/// lowercase ASCII letters, digits, and underscores, starting with a letter
/// or digit, 1 to 63 characters long.
///
/// # Why this exact shape
///
/// One connector slug ends up embedded in three different systems, each
/// with its own identifier charset:
///
/// - A **Postgres identifier** — the replication slot name (`{slug}_slot`)
///   and publication name (`{slug}_pub`). Unquoted Postgres identifiers are
///   case-folded and limited to alphanumerics/underscore starting with a
///   letter/underscore, and `NAMEDATALEN` caps the total length (63 bytes
///   by default, which is why the slug alone is capped shorter here to
///   leave room for the `_slot`/`_pub` suffixes).
/// - A **filesystem path segment** — `/debezium/data/{slug}-offsets.dat`
///   and the matching schema-history file. A slug containing `/`, `..`, or
///   a leading `-` can escape the intended data directory or be
///   misinterpreted as a flag.
/// - A **Kafka-style topic prefix** (`debezium.source.topic.prefix`).
///   Kafka topic names accept a wider charset than this, but there is no
///   reason to accept more than the other two constraints already allow.
///
/// Rather than pick one system's charset and escape/quote for the other
/// two — three different escaping schemes, three different ways to get it
/// wrong — this type validates the INTERSECTION of what all three accept
/// once, at construction, so every use site downstream is simply safe by
/// construction. That is simpler to review than any per-use-site escaping
/// discipline, and it closes the newline-injection and path-traversal
/// findings the unvalidated `String` version of this field allowed (a
/// slug containing `../` could escape `/debezium/data/`; a slug containing
/// a newline could inject arbitrary extra Debezium properties after
/// `debezium.source.slot.name=...`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConnectorSlug(String);

impl ConnectorSlug {
    /// Validate `value` against `^[a-z0-9][a-z0-9_]{0,62}$`.
    ///
    /// # Errors
    ///
    /// Returns [`CdcSpecError::InvalidConnectorSlug`] if `value` is empty,
    /// longer than 63 characters, starts with anything but a lowercase
    /// ASCII letter or digit, or contains anything but lowercase ASCII
    /// letters, digits, or `_`.
    pub fn new(value: &str) -> Result<Self, CdcSpecError> {
        let mut chars = value.chars();
        let starts_ok =
            matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit());
        let rest_ok = value.len() <= 63
            && value
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if starts_ok && rest_ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(CdcSpecError::InvalidConnectorSlug {
                slug: value.to_owned(),
            })
        }
    }

    /// Borrow the validated slug.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ConnectorSlug {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Reject a value containing a `\n`, `\r`, or any other ASCII/Unicode
/// control character before it is ever put into a connector config.
///
/// A Java `.properties` file (what a `Debezium` worker reads its
/// configuration from) has no quoting mechanism for a value: it is not a
/// structured format with escape sequences a writer can apply, it is
/// `key=value` up to the end of the line. So a newline embedded in a value
/// is not "data that needs escaping", it IS a line terminator — anything
/// after it is parsed as a brand-new `key=value` property, which lets a
/// single malicious field (a `host`, a `table` name, a resolved credential)
/// inject arbitrary additional Debezium configuration. Rejecting every
/// control character at construction, before formatting, means the
/// renderer ([`render_connect_config`]) never needs to escape
/// anything: every value it interpolates is already guaranteed
/// injection-free.
fn reject_control_characters(field: &'static str, value: &str) -> Result<(), CdcSpecError> {
    if value.chars().any(char::is_control) {
        return Err(CdcSpecError::ControlCharacterInField { field });
    }
    Ok(())
}

/// The directory the API writes each change-capture connector's source password
/// into, and `Kafka Connect`'s `DirectoryConfigProvider` reads it from
/// (`SRC-4` decision S2-a, ADR 0016 point 4). A memory-only volume, mounted
/// read-write in `lakehouse-api` and read-only in `kafka-connect`.
pub const CDC_SECRETS_DIR: &str = "/run/cdc-secrets";

/// `Debezium`'s `PostgreSQL` connector class, the one connector class this
/// build starts so far (`SRC-4` slice 2). `MySQL`, `SQL Server`, `MongoDB` and
/// `Oracle` are added by their slices, with the allowlist in the API's
/// `kafka_connect` module.
pub const POSTGRES_CONNECTOR_CLASS: &str = "io.debezium.connector.postgresql.PostgresConnector";

/// `Debezium`'s `MySQL` connector class (`SRC-4` slice 3).
pub const MYSQL_CONNECTOR_CLASS: &str = "io.debezium.connector.mysql.MySqlConnector";

/// `Debezium`'s `SQL Server` connector class (`SRC-4` slice 3).
pub const SQLSERVER_CONNECTOR_CLASS: &str = "io.debezium.connector.sqlserver.SqlServerConnector";

/// `Debezium`'s `MongoDB` connector class (`SRC-4` slice 5).
pub const MONGODB_CONNECTOR_CLASS: &str = "io.debezium.connector.mongodb.MongoDbConnector";

/// `Debezium`'s unwrap transform for `MongoDB` documents. The relational
/// `ExtractNewRecordState` does not understand a `MongoDB` change event.
const MONGODB_UNWRAP_CLASS: &str =
    "io.debezium.connector.mongodb.transforms.ExtractNewDocumentState";

/// The Kafka topic a `MySQL` or `SQL Server` connector keeps its schema history
/// in (`SRC-4` decision S3-b/S3-d). `Debezium` creates it with unlimited
/// retention; the broker's 7-day default must never apply to it, because a
/// connector whose history is gone cannot restart. It is never read by the
/// applier and never deleted with the connector (Kafka Connect has no topic API).
///
/// # Errors
///
/// [`CdcSpecError::InvalidConnectorId`] if `id` is not a plain identifier.
pub fn schema_history_topic(id: &str) -> Result<String, CdcSpecError> {
    if is_connector_id(id) {
        Ok(format!("cdc.{id}.schema-history"))
    } else {
        Err(CdcSpecError::InvalidConnectorId { id: id.to_owned() })
    }
}

/// Whether `id` is fit to be a Kafka Connect connector name, a topic prefix
/// and a file name in [`CDC_SECRETS_DIR`] at once: no path separator, no `:`
/// (the config provider's own separator), no control character.
fn is_connector_id(id: &str) -> bool {
    id.len() <= 63
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// The Kafka Connect connector name for registry connector `id`.
///
/// # Errors
///
/// [`CdcSpecError::InvalidConnectorId`] if `id` is not a plain identifier.
pub fn connect_connector_name(id: &str) -> Result<String, CdcSpecError> {
    if is_connector_id(id) {
        Ok(format!("lakehouse-{id}"))
    } else {
        Err(CdcSpecError::InvalidConnectorId { id: id.to_owned() })
    }
}

/// The `Kafka Connect` config-provider reference to registry connector
/// `id`'s password: `${dir:<secrets_dir>:<id>}`, `secrets_dir` being
/// [`CDC_SECRETS_DIR`] in a deployment. It is the only form of the password the
/// connector JSON ever holds.
///
/// # Errors
///
/// [`CdcSpecError::InvalidConnectorId`] if `id` is not a plain identifier;
/// [`CdcSpecError::InvalidSecretsDir`] if `secrets_dir` is not an absolute path
/// of plain characters (a `:` or `}` would end the reference early).
pub fn connect_password_reference(id: &str, secrets_dir: &str) -> Result<String, CdcSpecError> {
    let plain_dir = secrets_dir.starts_with('/')
        && secrets_dir
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'));
    if !plain_dir {
        return Err(CdcSpecError::InvalidSecretsDir);
    }
    if is_connector_id(id) {
        Ok(format!("${{dir:{secrets_dir}:{id}}}"))
    } else {
        Err(CdcSpecError::InvalidConnectorId { id: id.to_owned() })
    }
}

/// Whether `name` is a `PostgreSQL` slot or publication name `Debezium`
/// accepts: 1 to 63 of lowercase letters, digits and `_`.
fn is_replication_name(name: &str) -> bool {
    (1..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Whether `name` is `schema.table`, each part plain identifier characters.
fn split_table_name(name: &str) -> Option<(&str, &str)> {
    let (schema, table) = name.split_once('.')?;
    let plain = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$'))
    };
    (plain(schema) && plain(table)).then_some((schema, table))
}

/// Whether `list` is `host:port[,host:port]...` of plain characters (letters,
/// digits, `.`, `-`, `_`; a decimal port), as `schema.history.internal.kafka.
/// bootstrap.servers` takes it.
fn is_bootstrap_list(list: &str) -> bool {
    !list.is_empty()
        && list.split(',').all(|entry| {
            entry.split_once(':').is_some_and(|(host, port)| {
                !host.is_empty()
                    && host
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                    && !port.is_empty()
                    && port.bytes().all(|b| b.is_ascii_digit())
            })
        })
}

/// `MySQL` connector's `database.ssl.mode` for a dial's mode: `Debezium` spells
/// them `disabled`, `required`, `verify_ca` and `verify_identity`.
const fn mysql_ssl_mode(mode: CdcSslMode) -> &'static str {
    match mode {
        CdcSslMode::Disable => "disabled",
        CdcSslMode::Require => "required",
        CdcSslMode::VerifyCa => "verify_ca",
        CdcSslMode::VerifyFull => "verify_identity",
    }
}

/// `SQL Server` connector's `(database.encrypt, database.trustServerCertificate)`
/// for a dial's mode. `require` encrypts and takes the certificate as presented,
/// the meaning it has for `PostgreSQL`. `verify-ca` and `verify-full` both
/// encrypt and verify: the `SQL Server` driver cannot check the chain without
/// also checking the host name, so `verify-ca` is stricter here, never looser.
const fn mssql_tls(mode: CdcSslMode) -> (&'static str, &'static str) {
    match mode {
        CdcSslMode::Disable => ("false", "false"),
        CdcSslMode::Require => ("true", "true"),
        CdcSslMode::VerifyCa | CdcSslMode::VerifyFull => ("true", "false"),
    }
}

/// Render the `Kafka Connect` connector configuration for a change-capture
/// connector (`SRC-4` slice 2 decision S2-c, slice 3 decision S3-b): what the
/// API sends to `PUT /connectors/<name>/config`. It replaces the `Debezium
/// Server` properties template.
///
/// One renderer, three drivers. The keys every driver shares: the password
/// reference, `topic.prefix=cdc.<id>`, the JSON converters without schemas, the
/// `unwrap` transform with `delete.tombstone.handling.mode=rewrite` and
/// `add.fields=op,source.ts_ms`, `snapshot.mode=initial` and
/// `decimal.handling.mode=string`. `PostgreSQL` adds the dial's own slot and
/// publication; `MySQL` its `serverId`, the databases of the source objects and
/// the schema-history topic; `SQL Server` its database name and the
/// schema-history topic. The password is the config-provider reference of
/// [`connect_password_reference`], never a value, and every value is a string,
/// as `Kafka Connect` wants. `delete.tombstone.handling.mode=rewrite`:
/// `Debezium` 3.8 ignores `delete.handling.mode` and `drop.tombstones`, and
/// with them every delete reaches the topic as a message with no value
/// (`docs/plans/SRC-4-RESULT.md`).
///
/// With `verify-ca` and `verify-full` the worker uses its own trust store: this
/// renderer sets no root certificate.
///
/// `schema_history_bootstrap` is the broker address `MySQL` and `SQL Server`
/// connectors keep their schema history on; `PostgreSQL` ignores it.
///
/// # Errors
///
/// [`CdcSpecError::UnsupportedDriver`] for `Oracle`,
/// [`CdcSpecError::InvalidConnectorId`], [`CdcSpecError::InvalidSecretsDir`],
/// [`CdcSpecError::NoTables`], [`CdcSpecError::InvalidTableName`],
/// [`CdcSpecError::InvalidReplicationName`] (`PostgreSQL`),
/// [`CdcSpecError::NoServerId`] (`MySQL`),
/// [`CdcSpecError::NoSchemaHistoryBroker`] or
/// [`CdcSpecError::InvalidSchemaHistoryBroker`] (`MySQL`, `SQL Server`), or
/// [`CdcSpecError::ControlCharacterInField`] for a host, database or user
/// holding a control character.
pub fn render_connect_config(
    connector_id: &str,
    dial: &CdcDial,
    tables: &[&str],
    secrets_dir: &str,
    schema_history_bootstrap: Option<&str>,
) -> Result<BTreeMap<String, String>, CdcSpecError> {
    let password = connect_password_reference(connector_id, secrets_dir)?;
    if tables.is_empty() {
        return Err(CdcSpecError::NoTables);
    }
    reject_control_characters("host", &dial.host)?;
    reject_control_characters("database", &dial.database)?;
    reject_control_characters("user", &dial.user)?;

    // The first part of every table name: a schema (`PostgreSQL`, `SQL Server`)
    // or a database (`MySQL`).
    let mut first_parts: Vec<&str> = Vec::new();
    for name in tables {
        let (first, _) = split_table_name(name).ok_or_else(|| CdcSpecError::InvalidTableName {
            name: (*name).to_owned(),
        })?;
        if !first_parts.contains(&first) {
            first_parts.push(first);
        }
    }

    let mut entries: Vec<(&str, String)> = vec![
        ("tasks.max", "1".to_owned()),
        ("database.hostname", dial.host.clone()),
        ("database.port", dial.port.to_string()),
        ("database.user", dial.user.clone()),
        ("database.password", password),
        ("topic.prefix", format!("cdc.{connector_id}")),
        ("table.include.list", tables.join(",")),
        ("snapshot.mode", "initial".to_owned()),
        ("decimal.handling.mode", "string".to_owned()),
        (
            "key.converter",
            "org.apache.kafka.connect.json.JsonConverter".to_owned(),
        ),
        ("key.converter.schemas.enable", "false".to_owned()),
        (
            "value.converter",
            "org.apache.kafka.connect.json.JsonConverter".to_owned(),
        ),
        ("value.converter.schemas.enable", "false".to_owned()),
        ("transforms", "unwrap".to_owned()),
        (
            "transforms.unwrap.type",
            "io.debezium.transforms.ExtractNewRecordState".to_owned(),
        ),
        (
            "transforms.unwrap.delete.tombstone.handling.mode",
            "rewrite".to_owned(),
        ),
        ("transforms.unwrap.add.fields", "op,source.ts_ms".to_owned()),
    ];

    let driver_entries = match dial.driver {
        SqlDriver::Postgres => postgres_entries(dial, &first_parts)?,
        SqlDriver::Mysql => {
            mysql_entries(connector_id, dial, &first_parts, schema_history_bootstrap)?
        }
        SqlDriver::Mssql => mssql_entries(connector_id, dial, schema_history_bootstrap)?,
        SqlDriver::Oracle => return Err(CdcSpecError::UnsupportedDriver),
    };
    entries.extend(driver_entries);
    Ok(entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect())
}

/// The `PostgreSQL` connector's own keys: class, database, TLS, schemas, and the
/// dial's slot and publication.
fn postgres_entries(
    dial: &CdcDial,
    schemas: &[&str],
) -> Result<Vec<(&'static str, String)>, CdcSpecError> {
    let (Some(slot_name), Some(publication_name)) =
        (dial.slot_name.as_deref(), dial.publication_name.as_deref())
    else {
        // `Dial::parse` requires both for PostgreSQL; a hand-built dial without
        // them is refused the same way a bad name is.
        return Err(CdcSpecError::InvalidReplicationName { field: "slotName" });
    };
    for (field, value) in [
        ("slotName", slot_name),
        ("publicationName", publication_name),
    ] {
        if !is_replication_name(value) {
            return Err(CdcSpecError::InvalidReplicationName { field });
        }
    }
    Ok(vec![
        ("connector.class", POSTGRES_CONNECTOR_CLASS.to_owned()),
        ("database.dbname", dial.database.clone()),
        ("database.sslmode", dial.ssl_mode.as_debezium().to_owned()),
        ("schema.include.list", schemas.join(",")),
        ("plugin.name", "pgoutput".to_owned()),
        ("slot.name", slot_name.to_owned()),
        ("publication.name", publication_name.to_owned()),
        ("publication.autocreate.mode", "disabled".to_owned()),
    ])
}

/// The `MySQL` connector's own keys: class, server id, databases, TLS, and the
/// schema-history topic.
fn mysql_entries(
    connector_id: &str,
    dial: &CdcDial,
    databases: &[&str],
    bootstrap: Option<&str>,
) -> Result<Vec<(&'static str, String)>, CdcSpecError> {
    let server_id = dial.server_id.ok_or(CdcSpecError::NoServerId)?;
    let mut entries = vec![
        ("connector.class", MYSQL_CONNECTOR_CLASS.to_owned()),
        ("database.server.id", server_id.to_string()),
        ("database.include.list", databases.join(",")),
        (
            "database.ssl.mode",
            mysql_ssl_mode(dial.ssl_mode).to_owned(),
        ),
        // No topic named after the prefix alone: the applier reads the table
        // topics only.
        ("include.schema.changes", "false".to_owned()),
    ];
    entries.extend(schema_history_entries(connector_id, bootstrap)?);
    Ok(entries)
}

/// The `SQL Server` connector's own keys: class, database, TLS, and the
/// schema-history topic.
fn mssql_entries(
    connector_id: &str,
    dial: &CdcDial,
    bootstrap: Option<&str>,
) -> Result<Vec<(&'static str, String)>, CdcSpecError> {
    let (encrypt, trust) = mssql_tls(dial.ssl_mode);
    let mut entries = vec![
        ("connector.class", SQLSERVER_CONNECTOR_CLASS.to_owned()),
        ("database.names", dial.database.clone()),
        ("database.encrypt", encrypt.to_owned()),
        ("database.trustServerCertificate", trust.to_owned()),
    ];
    entries.extend(schema_history_entries(connector_id, bootstrap)?);
    Ok(entries)
}

/// The two schema-history keys of a `MySQL` or `SQL Server` connector.
fn schema_history_entries(
    connector_id: &str,
    bootstrap: Option<&str>,
) -> Result<[(&'static str, String); 2], CdcSpecError> {
    let bootstrap = bootstrap.ok_or(CdcSpecError::NoSchemaHistoryBroker)?;
    if !is_bootstrap_list(bootstrap) {
        return Err(CdcSpecError::InvalidSchemaHistoryBroker);
    }
    Ok([
        (
            "schema.history.internal.kafka.bootstrap.servers",
            bootstrap.to_owned(),
        ),
        (
            "schema.history.internal.kafka.topic",
            schema_history_topic(connector_id)?,
        ),
    ])
}

/// Whether `name` is one plain `MongoDB` name (a database or a collection) as
/// `collection.include.list` and a topic name take it.
fn is_plain_mongo_name(name: &str, max: usize) -> bool {
    (1..=max).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// Render the `Kafka Connect` connector configuration for a `MongoDB`
/// change-capture connector (`SRC-4` slice 5, decisions S5-b and S5-c).
///
/// `collections` are the source objects' names: one plain collection name each,
/// of the dial's database (discovery lists them that way, as the batch `MongoDB`
/// adapter reads them). The connector captures `<database>.<collection>` for
/// each, and `Debezium` writes the topic `cdc.<id>.<database>.<collection>`.
///
/// The keys: the `MongoDB` connector class; `mongodb.connection.string` built
/// from the dial's hosts and replica set and **no credential**; `mongodb.user`
/// and `mongodb.password`, the password as the config-provider reference of
/// [`connect_password_reference`], never a value; `mongodb.authsource` is the
/// dial's database; `capture.mode=change_streams_update_full`, so an update
/// carries the whole document, which the current-row mode needs; the JSON
/// converters without schemas; `snapshot.mode=initial`; and `MongoDB`'s own
/// `ExtractNewDocumentState` with `delete.tombstone.handling.mode=rewrite` and
/// `add.fields=op,source.ts_ms` (the keys the relational unwrap takes in
/// [`render_connect_config`], on the class that unwraps documents). These
/// shared keys are written out here, not drawn from that function, so the
/// relational renderer stays as slice 3 left it.
///
/// `directConnection=true` is set for a one-host dial and not otherwise: the
/// driver refuses it together with several seeds. Then the worker discovers
/// the other members of the replica set from the server and dials the addresses
/// the replica set advertises, which no check of ours has seen ("Limits" of the
/// feature page says so).
///
/// TLS: `disable` sets `mongodb.ssl.enabled=false`. The other three set it
/// `true`; `require` also sets `mongodb.ssl.invalid.hostname.allowed=true`, so
/// the host name need not match; `verify-ca` and `verify-full` set it `false`.
/// The certificate chain is checked against the worker's trust store in every
/// TLS mode: `Debezium` has no setting that takes a `MongoDB` certificate "as
/// presented", so `require` is weaker than `verify-full` by the host name only.
/// This renderer sets no root certificate.
///
/// # Errors
///
/// [`CdcSpecError::InvalidConnectorId`], [`CdcSpecError::InvalidSecretsDir`],
/// [`CdcSpecError::NoTables`], [`CdcSpecError::InvalidCollectionName`],
/// [`CdcSpecError::InvalidMongoHosts`], [`CdcSpecError::InvalidReplicaSet`],
/// [`CdcSpecError::InvalidMongoDatabase`], or
/// [`CdcSpecError::ControlCharacterInField`] for a user holding a control
/// character.
pub fn render_mongodb_connect_config(
    connector_id: &str,
    dial: &CdcMongoDial,
    collections: &[&str],
    secrets_dir: &str,
) -> Result<BTreeMap<String, String>, CdcSpecError> {
    let password = connect_password_reference(connector_id, secrets_dir)?;
    if collections.is_empty() {
        return Err(CdcSpecError::NoTables);
    }
    reject_control_characters("user", &dial.user)?;
    if !is_plain_mongo_name(&dial.database, 63) {
        return Err(CdcSpecError::InvalidMongoDatabase);
    }
    if dial.hosts.is_empty() || !is_bootstrap_list(&dial.hosts.join(",")) {
        return Err(CdcSpecError::InvalidMongoHosts);
    }
    let replica_set_ok = (1..=128).contains(&dial.replica_set.len())
        && dial
            .replica_set
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'));
    if !replica_set_ok {
        return Err(CdcSpecError::InvalidReplicaSet);
    }
    let mut include: Vec<String> = Vec::new();
    for name in collections {
        if !is_plain_mongo_name(name, 120) {
            return Err(CdcSpecError::InvalidCollectionName {
                name: (*name).to_owned(),
            });
        }
        include.push(format!("{}.{name}", dial.database));
    }

    let mut connection_string = format!(
        "mongodb://{}/?replicaSet={}",
        dial.hosts.join(","),
        dial.replica_set
    );
    if dial.hosts.len() == 1 {
        connection_string.push_str("&directConnection=true");
    }
    let (ssl_enabled, hostname_allowed) = match dial.ssl_mode {
        CdcSslMode::Disable => ("false", None),
        CdcSslMode::Require => ("true", Some("true")),
        CdcSslMode::VerifyCa | CdcSslMode::VerifyFull => ("true", Some("false")),
    };

    let mut entries: Vec<(&str, String)> = vec![
        ("connector.class", MONGODB_CONNECTOR_CLASS.to_owned()),
        ("tasks.max", "1".to_owned()),
        ("mongodb.connection.string", connection_string),
        ("mongodb.user", dial.user.clone()),
        ("mongodb.password", password),
        ("mongodb.authsource", dial.database.clone()),
        ("mongodb.ssl.enabled", ssl_enabled.to_owned()),
        ("topic.prefix", format!("cdc.{connector_id}")),
        ("collection.include.list", include.join(",")),
        ("capture.mode", "change_streams_update_full".to_owned()),
        ("snapshot.mode", "initial".to_owned()),
        (
            "key.converter",
            "org.apache.kafka.connect.json.JsonConverter".to_owned(),
        ),
        ("key.converter.schemas.enable", "false".to_owned()),
        (
            "value.converter",
            "org.apache.kafka.connect.json.JsonConverter".to_owned(),
        ),
        ("value.converter.schemas.enable", "false".to_owned()),
        ("transforms", "unwrap".to_owned()),
        ("transforms.unwrap.type", MONGODB_UNWRAP_CLASS.to_owned()),
        (
            "transforms.unwrap.delete.tombstone.handling.mode",
            "rewrite".to_owned(),
        ),
        ("transforms.unwrap.add.fields", "op,source.ts_ms".to_owned()),
    ];
    if let Some(allowed) = hostname_allowed {
        entries.push(("mongodb.ssl.invalid.hostname.allowed", allowed.to_owned()));
    }
    Ok(entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn pg_dial(ssl_mode: &str) -> CdcDial {
        serde_json::from_value(serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub",
            "sslMode": ssl_mode
        }))
        .unwrap()
    }

    #[test]
    fn connect_config_uses_the_dials_own_slot_publication_and_tls_mode() {
        let config = render_connect_config(
            "conn-orders-ab12",
            &pg_dial("verify-full"),
            &["public.orders", "sales.items"],
            CDC_SECRETS_DIR,
            None,
        )
        .unwrap();
        assert_eq!(config["connector.class"], POSTGRES_CONNECTOR_CLASS);
        assert_eq!(config["slot.name"], "orders_slot");
        assert_eq!(config["publication.name"], "orders_pub");
        assert_eq!(config["publication.autocreate.mode"], "disabled");
        assert_eq!(config["database.sslmode"], "verify-full");
        assert_eq!(config["database.hostname"], "db.internal");
        assert_eq!(config["table.include.list"], "public.orders,sales.items");
        assert_eq!(config["schema.include.list"], "public,sales");
        assert_eq!(config["topic.prefix"], "cdc.conn-orders-ab12");
        assert_eq!(config["snapshot.mode"], "initial");
        // Amendment 3: Debezium 3.8 ignores delete.handling.mode and drop.tombstones.
        assert_eq!(
            config["transforms.unwrap.delete.tombstone.handling.mode"],
            "rewrite"
        );
        assert_eq!(config["transforms.unwrap.add.fields"], "op,source.ts_ms");
        assert!(!config.contains_key("transforms.unwrap.delete.handling.mode"));
        assert!(!config.contains_key("transforms.unwrap.drop.tombstones"));
    }

    #[test]
    fn connect_config_holds_the_password_only_as_a_provider_reference() {
        let config = render_connect_config(
            "conn-orders-ab12",
            &pg_dial("require"),
            &["public.orders"],
            CDC_SECRETS_DIR,
            None,
        )
        .unwrap();
        assert_eq!(
            config["database.password"],
            "${dir:/run/cdc-secrets:conn-orders-ab12}"
        );
        assert_eq!(
            connect_password_reference("conn-a", "/tmp/x:y"),
            Err(CdcSpecError::InvalidSecretsDir)
        );
        assert_eq!(
            connect_connector_name("conn-orders-ab12").unwrap(),
            "lakehouse-conn-orders-ab12"
        );
    }

    #[test]
    fn connect_config_refuses_what_could_change_what_is_captured() {
        let dial = pg_dial("require");
        for table in [
            "orders",
            "public.orders,public.secrets",
            "public.or ders",
            "a.b.c",
            ".x",
        ] {
            assert!(
                matches!(
                    render_connect_config("conn-a", &dial, &[table], CDC_SECRETS_DIR, None),
                    Err(CdcSpecError::InvalidTableName { .. })
                ),
                "{table}"
            );
        }
        assert_eq!(
            render_connect_config("conn-a", &dial, &[], CDC_SECRETS_DIR, None),
            Err(CdcSpecError::NoTables)
        );
        for id in ["", "../x", "a:b", "-a", "a/b", "a\nb"] {
            assert!(
                matches!(
                    render_connect_config(id, &dial, &["public.t"], CDC_SECRETS_DIR, None),
                    Err(CdcSpecError::InvalidConnectorId { .. })
                ),
                "{id}"
            );
        }
    }

    #[test]
    fn connect_config_refuses_a_slot_name_debezium_would_reject_and_other_drivers() {
        let mut dial = pg_dial("require");
        dial.slot_name = Some("Orders-Slot".to_owned());
        assert!(matches!(
            render_connect_config("conn-a", &dial, &["public.t"], CDC_SECRETS_DIR, None),
            Err(CdcSpecError::InvalidReplicationName { field: "slotName" })
        ));
        let mut dial = pg_dial("require");
        dial.driver = SqlDriver::Oracle;
        assert_eq!(
            render_connect_config("conn-a", &dial, &["public.t"], CDC_SECRETS_DIR, None),
            Err(CdcSpecError::UnsupportedDriver)
        );
    }

    const BROKER: &str = "kafka:9092";

    fn other_dial(driver: &str, ssl_mode: &str) -> CdcDial {
        let mut json = serde_json::json!({
            "driver": driver, "host": "db.internal", "port": 3306,
            "database": "orders", "user": "app_reader", "sslMode": ssl_mode
        });
        if driver == "mysql" {
            json["serverId"] = serde_json::json!(5400);
        }
        serde_json::from_value(json).unwrap()
    }

    fn render_other(driver: &str, ssl_mode: &str, tables: &[&str]) -> BTreeMap<String, String> {
        render_connect_config(
            "conn-x1",
            &other_dial(driver, ssl_mode),
            tables,
            CDC_SECRETS_DIR,
            Some(BROKER),
        )
        .unwrap()
    }

    /// The keys every driver shares, so a change to the shared part is seen once.
    fn assert_shared_keys(config: &BTreeMap<String, String>) {
        assert_eq!(
            config["database.password"],
            "${dir:/run/cdc-secrets:conn-x1}"
        );
        assert_eq!(config["topic.prefix"], "cdc.conn-x1");
        assert_eq!(config["snapshot.mode"], "initial");
        assert_eq!(config["decimal.handling.mode"], "string");
        assert_eq!(config["tasks.max"], "1");
        assert_eq!(
            config["transforms.unwrap.delete.tombstone.handling.mode"],
            "rewrite"
        );
        assert_eq!(config["transforms.unwrap.add.fields"], "op,source.ts_ms");
        assert_eq!(config["value.converter.schemas.enable"], "false");
        assert!(!config.values().any(|v| v.contains("s3cret")));
    }

    #[test]
    fn mysql_config_has_exactly_its_keys() {
        let config = render_other(
            "mysql",
            "require",
            &["shop.orders", "shop.items", "crm.users"],
        );
        assert_shared_keys(&config);
        assert_eq!(config["connector.class"], MYSQL_CONNECTOR_CLASS);
        assert_eq!(config["database.server.id"], "5400");
        assert_eq!(config["database.include.list"], "shop,crm");
        assert_eq!(
            config["table.include.list"],
            "shop.orders,shop.items,crm.users"
        );
        assert_eq!(config["include.schema.changes"], "false");
        assert_eq!(
            config["schema.history.internal.kafka.bootstrap.servers"],
            BROKER
        );
        assert_eq!(
            config["schema.history.internal.kafka.topic"],
            "cdc.conn-x1.schema-history"
        );
        // None of PostgreSQL's keys.
        for key in [
            "slot.name",
            "publication.name",
            "plugin.name",
            "database.sslmode",
            "database.dbname",
            "database.names",
        ] {
            assert!(!config.contains_key(key), "{key}");
        }
    }

    #[test]
    fn sql_server_config_has_exactly_its_keys() {
        let config = render_other("mssql", "require", &["dbo.orders", "sales.items"]);
        assert_shared_keys(&config);
        assert_eq!(config["connector.class"], SQLSERVER_CONNECTOR_CLASS);
        assert_eq!(config["database.names"], "orders");
        assert_eq!(config["table.include.list"], "dbo.orders,sales.items");
        assert_eq!(
            config["schema.history.internal.kafka.bootstrap.servers"],
            BROKER
        );
        assert_eq!(
            config["schema.history.internal.kafka.topic"],
            "cdc.conn-x1.schema-history"
        );
        for key in [
            "slot.name",
            "database.server.id",
            "database.include.list",
            "database.sslmode",
            "database.ssl.mode",
            "include.schema.changes",
        ] {
            assert!(!config.contains_key(key), "{key}");
        }
    }

    #[test]
    fn each_ssl_mode_maps_to_the_right_setting_per_driver() {
        for (mode, mysql, encrypt, trust) in [
            ("disable", "disabled", "false", "false"),
            ("require", "required", "true", "true"),
            ("verify-ca", "verify_ca", "true", "false"),
            ("verify-full", "verify_identity", "true", "false"),
        ] {
            let config = render_other("mysql", mode, &["shop.orders"]);
            assert_eq!(config["database.ssl.mode"], mysql, "{mode}");
            let config = render_other("mssql", mode, &["dbo.orders"]);
            assert_eq!(config["database.encrypt"], encrypt, "{mode}");
            assert_eq!(config["database.trustServerCertificate"], trust, "{mode}");
        }
    }

    #[test]
    fn mysql_and_sql_server_refuse_a_bad_table_name_and_a_missing_or_bad_history_broker() {
        for driver in ["mysql", "mssql"] {
            let dial = other_dial(driver, "require");
            for table in ["orders", "a.b.c", "x.y,z.w", "x. y"] {
                assert!(
                    matches!(
                        render_connect_config(
                            "conn-x1",
                            &dial,
                            &[table],
                            CDC_SECRETS_DIR,
                            Some(BROKER)
                        ),
                        Err(CdcSpecError::InvalidTableName { .. })
                    ),
                    "{driver} {table}"
                );
            }
            assert_eq!(
                render_connect_config("conn-x1", &dial, &["a.b"], CDC_SECRETS_DIR, None),
                Err(CdcSpecError::NoSchemaHistoryBroker)
            );
            for bad in [
                "",
                "kafka",
                "kafka:",
                ":9092",
                "ka fka:9092",
                "k:9092,",
                "k:9092\nx:1",
            ] {
                assert_eq!(
                    render_connect_config("conn-x1", &dial, &["a.b"], CDC_SECRETS_DIR, Some(bad)),
                    Err(CdcSpecError::InvalidSchemaHistoryBroker),
                    "{bad:?}"
                );
            }
        }
        let mut dial = other_dial("mysql", "require");
        dial.server_id = None;
        assert_eq!(
            render_connect_config("conn-x1", &dial, &["a.b"], CDC_SECRETS_DIR, Some(BROKER)),
            Err(CdcSpecError::NoServerId)
        );
    }

    #[test]
    fn postgres_ignores_the_schema_history_broker() {
        let with = render_connect_config(
            "c1",
            &pg_dial("require"),
            &["public.t"],
            CDC_SECRETS_DIR,
            Some(BROKER),
        )
        .unwrap();
        let without = render_connect_config(
            "c1",
            &pg_dial("require"),
            &["public.t"],
            CDC_SECRETS_DIR,
            None,
        )
        .unwrap();
        assert_eq!(with, without);
        assert!(!with.keys().any(|k| k.starts_with("schema.history")));
    }

    fn col(name: &str, type_name: &str) -> SourceColumn {
        SourceColumn {
            name: name.to_owned(),
            type_name: type_name.to_owned(),
        }
    }

    #[test]
    fn scalar_columns_are_accepted() {
        let columns = [
            col("id", "bigint"),
            col("amount", "numeric(12,2)"),
            col("payload", "jsonb"),
            col("created_at", "timestamptz"),
            col("label", "character varying"),
        ];
        assert!(reject_unsupported_column_types(&columns).is_ok());
    }

    #[test]
    fn postgres_array_display_is_rejected() {
        let columns = [col("tags", "text[]")];
        let err = reject_unsupported_column_types(&columns).unwrap_err();
        assert_eq!(err.column, "tags");
    }

    /// The R7 gate's relationship to something real.
    ///
    /// `reject_unsupported_column_types` has no product caller: nothing in
    /// this build introspects a source schema, because the connector
    /// registry stores a host but no table name, so there is nothing to
    /// introspect at registration or probe time. Rather than invent a caller
    /// (a worse outcome than none — a fabricated call site would imply a
    /// guarantee the product does not make), this pins the gate to the one
    /// source schema the stack actually ships: `p5_cdc.orders`, created by
    /// `docker-compose.yml`'s CDC seed and streamed by the demo connector.
    ///
    /// So the gate is exercised against the real columns that really flow
    /// through Debezium into Bronze. If someone adds a `jsonb` or array
    /// column to that seed, this fails and says so — which is exactly the
    /// moment the gate was written to catch, arriving as a test failure
    /// rather than as unreadable Bronze data discovered later.
    #[test]
    fn the_shipped_cdc_source_schema_passes_the_r7_gate() {
        // Mirrors docker-compose.yml's `CREATE TABLE p5_cdc.orders`.
        let columns = [
            col("id", "bigint"),
            col("customer", "text"),
            col("amount", "numeric(12,2)"),
            col("updated_at", "timestamptz"),
        ];
        assert!(
            reject_unsupported_column_types(&columns).is_ok(),
            "the demo CDC table's own columns must pass the gate that decides \
             whether a connector may stream into Bronze"
        );
    }

    /// The same schema with one realistic addition that MUST be refused, so
    /// the test above cannot pass by the gate being permissive.
    #[test]
    fn adding_a_nested_column_to_that_schema_is_refused() {
        let columns = [
            col("id", "bigint"),
            col("customer", "text"),
            col("metadata", "jsonb[]"),
        ];
        let err =
            reject_unsupported_column_types(&columns).expect_err("an array column must be refused");
        assert_eq!(err.column, "metadata");
    }

    #[test]
    fn nested_struct_is_rejected() {
        for type_name in ["struct", "record", "composite", "map<string,string>", "ROW"] {
            let columns = [col("nested", type_name)];
            assert!(
                reject_unsupported_column_types(&columns).is_err(),
                "{type_name} should be rejected"
            );
        }
    }

    #[test]
    fn first_offending_column_is_reported() {
        let columns = [
            col("ok", "int"),
            col("bad", "struct"),
            col("also_bad", "array"),
        ];
        let err = reject_unsupported_column_types(&columns).unwrap_err();
        assert_eq!(err.column, "bad");
    }

    // ---- ConnectorSlug ----

    #[test]
    fn connector_slug_accepts_the_documented_shape() {
        for ok in ["a", "a1", "orders_pg", "p5cdc", "z".repeat(63).as_str()] {
            assert!(
                ConnectorSlug::new(ok).is_ok(),
                "{ok:?} should be a valid slug"
            );
        }
    }

    #[test]
    fn connector_slug_rejects_path_traversal() {
        let err = ConnectorSlug::new("../x").unwrap_err();
        assert!(matches!(err, CdcSpecError::InvalidConnectorSlug { .. }));
    }

    #[test]
    fn connector_slug_rejects_embedded_newline() {
        assert!(ConnectorSlug::new("a\nb").is_err());
    }

    #[test]
    fn connector_slug_rejects_empty() {
        assert!(ConnectorSlug::new("").is_err());
    }

    #[test]
    fn connector_slug_rejects_too_long() {
        let too_long = "a".repeat(64);
        assert!(ConnectorSlug::new(&too_long).is_err());
        // The boundary itself (63 chars) must still be accepted.
        let exactly_63 = "a".repeat(63);
        assert!(ConnectorSlug::new(&exactly_63).is_ok());
    }

    #[test]
    fn connector_slug_rejects_uppercase() {
        assert!(ConnectorSlug::new("Orders").is_err());
        assert!(ConnectorSlug::new("orders_PG").is_err());
    }

    #[test]
    fn connector_slug_rejects_leading_underscore_and_hyphen() {
        assert!(ConnectorSlug::new("_orders").is_err());
        assert!(ConnectorSlug::new("-orders").is_err());
    }

    fn mongo_dial(hosts: &[&str], ssl_mode: &str) -> CdcMongoDial {
        serde_json::from_value(serde_json::json!({
            "driver": "mongodb", "hosts": hosts, "database": "shop",
            "user": "app_reader", "replicaSet": "rs0", "sslMode": ssl_mode
        }))
        .unwrap()
    }

    fn render_mongo(
        hosts: &[&str],
        ssl_mode: &str,
        collections: &[&str],
    ) -> BTreeMap<String, String> {
        render_mongodb_connect_config(
            "conn-m1",
            &mongo_dial(hosts, ssl_mode),
            collections,
            CDC_SECRETS_DIR,
        )
        .unwrap()
    }

    #[test]
    fn mongodb_config_has_exactly_the_keys_slice_five_decided() {
        let config = render_mongo(&["mongo-a.internal:27017"], "require", &["orders", "items"]);
        let expected: BTreeMap<String, String> = [
            ("connector.class", MONGODB_CONNECTOR_CLASS),
            ("tasks.max", "1"),
            (
                "mongodb.connection.string",
                "mongodb://mongo-a.internal:27017/?replicaSet=rs0&directConnection=true",
            ),
            ("mongodb.user", "app_reader"),
            ("mongodb.password", "${dir:/run/cdc-secrets:conn-m1}"),
            ("mongodb.authsource", "shop"),
            ("mongodb.ssl.enabled", "true"),
            ("mongodb.ssl.invalid.hostname.allowed", "true"),
            ("topic.prefix", "cdc.conn-m1"),
            ("collection.include.list", "shop.orders,shop.items"),
            ("capture.mode", "change_streams_update_full"),
            ("snapshot.mode", "initial"),
            (
                "key.converter",
                "org.apache.kafka.connect.json.JsonConverter",
            ),
            ("key.converter.schemas.enable", "false"),
            (
                "value.converter",
                "org.apache.kafka.connect.json.JsonConverter",
            ),
            ("value.converter.schemas.enable", "false"),
            ("transforms", "unwrap"),
            (
                "transforms.unwrap.type",
                "io.debezium.connector.mongodb.transforms.ExtractNewDocumentState",
            ),
            (
                "transforms.unwrap.delete.tombstone.handling.mode",
                "rewrite",
            ),
            ("transforms.unwrap.add.fields", "op,source.ts_ms"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        assert_eq!(config, expected);
        // Amendment 3's lesson holds for the other class: the ignored keys are not written.
        assert!(!config.contains_key("transforms.unwrap.delete.handling.mode"));
        assert!(!config.contains_key("transforms.unwrap.drop.tombstones"));
        // No SQL-only key leaks in.
        assert!(!config.contains_key("database.hostname"));
        assert!(!config.contains_key("decimal.handling.mode"));
    }

    #[test]
    fn mongodb_config_holds_no_password_value_and_no_credential_in_the_connection_string() {
        let config = render_mongo(
            &["mongo-a.internal:27017", "mongo-b.internal:27017"],
            "verify-full",
            &["orders"],
        );
        assert_eq!(
            config["mongodb.connection.string"],
            "mongodb://mongo-a.internal:27017,mongo-b.internal:27017/?replicaSet=rs0"
        );
        assert!(!config["mongodb.connection.string"].contains('@'));
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("${dir:/run/cdc-secrets:conn-m1}"));
        // The only place the password appears is the reference; nothing else carries a secret.
        assert_eq!(json.matches("conn-m1}").count(), 1);
    }

    #[test]
    fn mongodb_config_sets_direct_connection_for_one_host_only() {
        let one = render_mongo(&["mongo-a.internal:27017"], "require", &["orders"]);
        assert!(one["mongodb.connection.string"].ends_with("&directConnection=true"));
        let two = render_mongo(
            &["mongo-a.internal:27017", "mongo-b.internal:27017"],
            "require",
            &["orders"],
        );
        assert!(!two["mongodb.connection.string"].contains("directConnection"));
    }

    #[test]
    fn mongodb_config_maps_each_ssl_mode() {
        let setting = |mode: &str| {
            let config = render_mongo(&["mongo-a.internal:27017"], mode, &["orders"]);
            (
                config["mongodb.ssl.enabled"].clone(),
                config.get("mongodb.ssl.invalid.hostname.allowed").cloned(),
            )
        };
        assert_eq!(setting("disable"), ("false".to_owned(), None));
        assert_eq!(
            setting("require"),
            ("true".to_owned(), Some("true".to_owned()))
        );
        assert_eq!(
            setting("verify-ca"),
            ("true".to_owned(), Some("false".to_owned()))
        );
        assert_eq!(
            setting("verify-full"),
            ("true".to_owned(), Some("false".to_owned()))
        );
    }

    #[test]
    fn mongodb_config_refuses_what_could_change_what_is_captured_or_dialed() {
        let dial = mongo_dial(&["mongo-a.internal:27017"], "require");
        let render = |dial: &CdcMongoDial, collections: &[&str]| {
            render_mongodb_connect_config("conn-m1", dial, collections, CDC_SECRETS_DIR)
        };
        // A collection is one plain name: not `database.collection`, not a regex, not a list.
        for name in ["shop.orders", "or ders", "a,b", ".x", "a*", "", "a$b"] {
            assert!(
                matches!(
                    render(&dial, &[name]),
                    Err(CdcSpecError::InvalidCollectionName { .. })
                ),
                "{name:?}"
            );
        }
        assert_eq!(render(&dial, &[]), Err(CdcSpecError::NoTables));
        // Hosts, replica set and database written into the connection string.
        for hosts in [
            vec!["mongo-a.internal"],
            vec!["mongo-a.internal:27017/admin"],
            vec!["mongo-a.internal:27017?x=y"],
            vec!["u:p@mongo-a.internal:27017"],
            vec![],
        ] {
            let mut bad = dial.clone();
            bad.hosts = hosts.iter().map(|h| (*h).to_owned()).collect();
            assert_eq!(
                render(&bad, &["orders"]),
                Err(CdcSpecError::InvalidMongoHosts)
            );
        }
        let mut bad = dial.clone();
        bad.replica_set = "rs0&authSource=admin".to_owned();
        assert_eq!(
            render(&bad, &["orders"]),
            Err(CdcSpecError::InvalidReplicaSet)
        );
        let mut bad = dial.clone();
        bad.database = "shop,admin".to_owned();
        assert_eq!(
            render(&bad, &["orders"]),
            Err(CdcSpecError::InvalidMongoDatabase)
        );
        let mut bad = dial.clone();
        bad.user = "a\nb".to_owned();
        assert!(matches!(
            render(&bad, &["orders"]),
            Err(CdcSpecError::ControlCharacterInField { field: "user" })
        ));
        for id in ["", "../x", "a:b", "-a", "a/b"] {
            assert!(
                matches!(
                    render_mongodb_connect_config(id, &dial, &["orders"], CDC_SECRETS_DIR),
                    Err(CdcSpecError::InvalidConnectorId { .. })
                ),
                "{id}"
            );
        }
    }
}
