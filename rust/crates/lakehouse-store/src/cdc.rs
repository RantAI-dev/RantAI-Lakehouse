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

use crate::ingest_spec::{CdcDial, SqlDriver};

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
    /// for yet (`MySQL`, `SQL Server` and `Oracle` come with their slices).
    #[error("change capture is not built for this source driver yet")]
    UnsupportedDriver,
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

/// Render the `Kafka Connect` connector configuration for a `PostgreSQL`
/// change-capture connector (`SRC-4` slice 2, decision S2-c): what the API
/// sends to `PUT /connectors/<name>/config`. It replaces the `Debezium Server`
/// properties template.
///
/// The slot and publication are the dial's own (`slotName`, `publicationName`),
/// the tables are the connector's source objects, the password is the
/// config-provider reference of [`connect_password_reference`], never a value,
/// and every value is a string, as `Kafka Connect` wants. The unwrap transform
/// writes `delete.tombstone.handling.mode=rewrite`: `Debezium` 3.8 ignores
/// `delete.handling.mode` and `drop.tombstones`, and with them every delete
/// reaches the topic as a message with no value (`docs/plans/SRC-4-RESULT.md`).
///
/// With `verify-ca` and `verify-full` the worker uses its own trust store:
/// this renderer sets no root certificate.
///
/// # Errors
///
/// [`CdcSpecError::UnsupportedDriver`] for a driver other than `PostgreSQL`,
/// [`CdcSpecError::InvalidConnectorId`], [`CdcSpecError::InvalidSecretsDir`],
/// [`CdcSpecError::NoTables`],
/// [`CdcSpecError::InvalidTableName`], [`CdcSpecError::InvalidReplicationName`]
/// or [`CdcSpecError::ControlCharacterInField`] for a host, database or user
/// holding a control character.
pub fn render_connect_config(
    connector_id: &str,
    dial: &CdcDial,
    tables: &[&str],
    secrets_dir: &str,
) -> Result<BTreeMap<String, String>, CdcSpecError> {
    if dial.driver != SqlDriver::Postgres {
        return Err(CdcSpecError::UnsupportedDriver);
    }
    let password = connect_password_reference(connector_id, secrets_dir)?;
    if tables.is_empty() {
        return Err(CdcSpecError::NoTables);
    }
    let (Some(slot_name), Some(publication_name)) =
        (dial.slot_name.as_deref(), dial.publication_name.as_deref())
    else {
        // `Dial::parse` requires both for PostgreSQL; a hand-built dial
        // without them is refused the same way a bad name is.
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
    reject_control_characters("host", &dial.host)?;
    reject_control_characters("database", &dial.database)?;
    reject_control_characters("user", &dial.user)?;

    let mut schemas: Vec<&str> = Vec::new();
    for name in tables {
        let (schema, _) = split_table_name(name).ok_or_else(|| CdcSpecError::InvalidTableName {
            name: (*name).to_owned(),
        })?;
        if !schemas.contains(&schema) {
            schemas.push(schema);
        }
    }

    let entries = [
        ("connector.class", POSTGRES_CONNECTOR_CLASS.to_owned()),
        ("tasks.max", "1".to_owned()),
        ("database.hostname", dial.host.clone()),
        ("database.port", dial.port.to_string()),
        ("database.user", dial.user.clone()),
        ("database.password", password),
        ("database.dbname", dial.database.clone()),
        ("database.sslmode", dial.ssl_mode.as_debezium().to_owned()),
        ("topic.prefix", format!("cdc.{connector_id}")),
        ("schema.include.list", schemas.join(",")),
        ("table.include.list", tables.join(",")),
        ("plugin.name", "pgoutput".to_owned()),
        ("slot.name", slot_name.to_owned()),
        ("publication.name", publication_name.to_owned()),
        ("publication.autocreate.mode", "disabled".to_owned()),
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
                    render_connect_config("conn-a", &dial, &[table], CDC_SECRETS_DIR),
                    Err(CdcSpecError::InvalidTableName { .. })
                ),
                "{table}"
            );
        }
        assert_eq!(
            render_connect_config("conn-a", &dial, &[], CDC_SECRETS_DIR),
            Err(CdcSpecError::NoTables)
        );
        for id in ["", "../x", "a:b", "-a", "a/b", "a\nb"] {
            assert!(
                matches!(
                    render_connect_config(id, &dial, &["public.t"], CDC_SECRETS_DIR),
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
            render_connect_config("conn-a", &dial, &["public.t"], CDC_SECRETS_DIR),
            Err(CdcSpecError::InvalidReplicationName { field: "slotName" })
        ));
        let mut dial = pg_dial("require");
        dial.driver = SqlDriver::Mysql;
        assert_eq!(
            render_connect_config("conn-a", &dial, &["public.t"], CDC_SECRETS_DIR),
            Err(CdcSpecError::UnsupportedDriver)
        );
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
}
