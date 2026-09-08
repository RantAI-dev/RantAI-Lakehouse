//! Connector registry -> Debezium Server runtime config (ADR 0007), and the
//! R7 schema-evolution gate ADR 0006 named but deferred to this phase:
//! nested struct/array/map source columns are rejected here, at connector
//! registration, not discovered later at read time as an unreadable Bronze
//! column.
//!
//! # What this module does NOT do
//!
//! It does not call out to a source database to discover its schema, does
//! not open a network connection anywhere, and does not persist anything.
//! [`reject_unsupported_column_types`] is a pure function over a
//! caller-supplied column list (whatever discovers that list — a future
//! console "test connection + inspect schema" flow — is out of scope here;
//! `lakehouse-api`'s `connector_probe` module added a real `PostgreSQL`/S3
//! connectivity *test* in P6, but it does not do schema discovery, which
//! stays exactly as out of scope as before). [`render_debezium_properties`]
//! is a pure string-template
//! function over already-resolved, already-validated inputs; the caller
//! resolves `secretRef` via [`lakehouse_core::secret::SecretResolver`]
//! (ADR 0002) before calling it. Actually running a generated config
//! (creating a new `debezium-server` process/container per connector) is
//! deployment orchestration, not this crate's job — see ADR 0007's "what
//! P5 does NOT do" for why that stays out of scope this phase.

use lakehouse_core::secret::SecretValue;

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

/// Everything that can go wrong building a [`DebeziumSourceSpec`] or
/// rendering it via [`render_debezium_properties`] — always a construction-
/// or render-time input problem, never something discovered by talking to
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

    /// One of the values that gets interpolated into a `.properties` file
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
/// control character before it is ever interpolated into a `.properties`
/// file.
///
/// A Java `.properties` file (what `debezium-server` reads its
/// configuration from) has no quoting mechanism for a value: it is not a
/// structured format with escape sequences a writer can apply, it is
/// `key=value` up to the end of the line. So a newline embedded in a value
/// is not "data that needs escaping", it IS a line terminator — anything
/// after it is parsed as a brand-new `key=value` property, which lets a
/// single malicious field (a `host`, a `table` name, a resolved credential)
/// inject arbitrary additional Debezium configuration. Rejecting every
/// control character at construction, before formatting, means the
/// template in [`render_debezium_properties`] never needs to escape
/// anything: every value it interpolates is already guaranteed
/// injection-free.
fn reject_control_characters(field: &'static str, value: &str) -> Result<(), CdcSpecError> {
    if value.chars().any(char::is_control) {
        return Err(CdcSpecError::ControlCharacterInField { field });
    }
    Ok(())
}

/// Everything [`render_debezium_properties`] needs to describe the
/// `Postgres` logical-replication source side of one connector.
///
/// Every field other than `connector_slug`/`database_port` is validated by
/// [`Self::new`] to reject control characters (see
/// [`reject_control_characters`]) before this type can be constructed at
/// all — there is no way to obtain a `DebeziumSourceSpec` whose fields
/// haven't already been checked, which is what makes
/// [`render_debezium_properties`] safe to format without any further
/// escaping.
#[derive(Debug, Clone)]
pub struct DebeziumSourceSpec {
    connector_slug: ConnectorSlug,
    database_hostname: String,
    database_port: u16,
    database_name: String,
    database_user: String,
    schema_qualified_table: String,
}

impl DebeziumSourceSpec {
    /// Build a validated source spec.
    ///
    /// `connector_slug` is a short, unique name for this connector's
    /// `Debezium` topic prefix, replication slot, and publication —
    /// derived from the `connector.id` slug (ADR 0004's
    /// `sanitize_table_name` shape), never user-supplied verbatim, so it is
    /// always a safe identifier once wrapped in [`ConnectorSlug`].
    /// `database_hostname`/`database_user`/`database_name` are the source
    /// `Postgres` connection's host/user/dbname (never logged by this
    /// function's caller per `connectors.rs`'s guarantee 2).
    /// `schema_qualified_table` is the schema-qualified table to capture,
    /// e.g. `public.orders`.
    ///
    /// # Errors
    ///
    /// Returns [`CdcSpecError::ControlCharacterInField`] if
    /// `database_hostname`, `database_name`, `database_user`, or
    /// `schema_qualified_table` contains a newline or other control
    /// character — see [`reject_control_characters`].
    pub fn new(
        connector_slug: ConnectorSlug,
        database_hostname: impl Into<String>,
        database_port: u16,
        database_name: impl Into<String>,
        database_user: impl Into<String>,
        schema_qualified_table: impl Into<String>,
    ) -> Result<Self, CdcSpecError> {
        let database_hostname = database_hostname.into();
        let database_name = database_name.into();
        let database_user = database_user.into();
        let schema_qualified_table = schema_qualified_table.into();
        reject_control_characters("database_hostname", &database_hostname)?;
        reject_control_characters("database_name", &database_name)?;
        reject_control_characters("database_user", &database_user)?;
        reject_control_characters("schema_qualified_table", &schema_qualified_table)?;
        Ok(Self {
            connector_slug,
            database_hostname,
            database_port,
            database_name,
            database_user,
            schema_qualified_table,
        })
    }
}

/// Everything [`render_debezium_properties`] needs to describe the Bronze
/// Iceberg sink side, shared across every connector in one deployment
/// (`Lakekeeper` warehouse + `RustFS`/`SeaweedFS` endpoint, per ADR 0003/ADR
/// 0004) rather than per-connector.
#[derive(Debug, Clone)]
pub struct IcebergSinkSpec {
    /// `Lakekeeper`'s REST catalog URI, e.g. `http://lakekeeper:8181/catalog`.
    pub catalog_uri: String,
    /// The tenant's `Lakekeeper` warehouse name (ADR 0003).
    pub warehouse: String,
    /// The S3-compatible endpoint backing that warehouse (`RustFS` or
    /// `SeaweedFS`, per ADR 0004/G2).
    pub s3_endpoint: String,
    /// The static bearer token `Lakekeeper`'s REST catalog authenticates
    /// with (R1/ADR 0011 — the `debezium` principal's pre-minted token,
    /// mounted read-only into the `debezium-server` container and read
    /// from disk at startup). `None` for a deployment that runs `Lakekeeper`
    /// without authorization enabled.
    pub catalog_token: Option<SecretValue>,
}

/// Render a `debezium-server-iceberg` `application.properties` body for one
/// connector — the control-plane -> runtime path ADR 0007 designs. Upsert
/// mode, initial snapshot then streaming, additive-only schema evolution
/// (`Debezium`/Iceberg's own default — see ADR 0006), file-based offset/
/// schema-history storage (P5-RESULT.md's measured trap: the
/// Iceberg-backed alternatives default ON and silently create two more
/// catalog tables that have nothing to do with Bronze), and
/// `publication.autocreate.mode=disabled` (matching the checked-in demo
/// connector: the publication is provisioned out of band, not
/// autocreated, so a re-run never silently narrows/widens it).
///
/// Every secret value is interpolated into the returned `String` — by
/// construction, since a `Debezium` Server properties file has no
/// indirection mechanism for secrets. The caller is responsible for never
/// logging the returned string; this function itself never does (its
/// return value is not `Debug`-printed anywhere in this crate).
///
/// # Callers
///
/// As of this commit, nothing in `lakehouse-api`/`lakehouse-store` calls
/// this function outside its own tests: the compose stack
/// (`docker-compose.yml`'s `debezium-server` service) ships a
/// hand-written `application.properties` heredoc for the one demo
/// connector this phase supports (ADR 0007: "no dynamic per-connector
/// provisioning" this phase). This function is the mechanism a future
/// provisioning flow (one that takes a connector registration and turns it
/// into a running `debezium-server` process) will call instead of that
/// hand-written file. Until that flow exists,
/// [`demo_connector_properties_match_the_checked_in_compose_file`] is what
/// keeps this function honest: it renders a spec built from the SAME
/// literal values the compose heredoc uses and asserts the two produce the
/// same set of properties, so the renderer and the actually-deployed
/// config cannot silently drift apart.
///
/// # Errors
///
/// Returns [`CdcSpecError::ControlCharacterInField`] if `database_password`,
/// `s3_access_key`, `s3_secret_key`, or `sink.catalog_token` contains a
/// newline or other control character. `source`'s and `sink`'s own string
/// fields are already validated ([`DebeziumSourceSpec::new`]); the
/// credential values handed to this call are the last unvalidated inputs,
/// since they are resolved by the caller immediately before calling this
/// function (see the module doc comment) rather than stored in either spec
/// type.
#[must_use = "this only builds a properties string; the caller must still write/deliver it"]
pub fn render_debezium_properties(
    source: &DebeziumSourceSpec,
    sink: &IcebergSinkSpec,
    database_password: &SecretValue,
    s3_access_key: &SecretValue,
    s3_secret_key: &SecretValue,
) -> Result<String, CdcSpecError> {
    reject_control_characters("database_password", database_password.expose_secret())?;
    reject_control_characters("s3_access_key", s3_access_key.expose_secret())?;
    reject_control_characters("s3_secret_key", s3_secret_key.expose_secret())?;
    if let Some(token) = &sink.catalog_token {
        reject_control_characters("catalog_token", token.expose_secret())?;
    }

    let slug = source.connector_slug.as_str();
    let token_line = sink
        .catalog_token
        .as_ref()
        .map(|token| format!("debezium.sink.iceberg.token={}\n", token.expose_secret()))
        .unwrap_or_default();
    // `schema.include.list` is derived from `schema_qualified_table` rather
    // than taking a separate field: the two must always agree (Debezium
    // rejects captures where the table isn't inside the included schema),
    // so deriving one from the other removes an entire class of "these two
    // fields disagree" misconfiguration.
    let schema = source
        .schema_qualified_table
        .split_once('.')
        .map_or(source.schema_qualified_table.as_str(), |(schema, _)| schema);

    Ok(format!(
        "debezium.sink.type=iceberg\n\
         debezium.sink.iceberg.catalog-name=lakekeeper\n\
         debezium.sink.iceberg.type=rest\n\
         debezium.sink.iceberg.uri={catalog_uri}\n\
         debezium.sink.iceberg.warehouse={warehouse}\n\
         {token_line}\
         debezium.sink.iceberg.io-impl=org.apache.iceberg.aws.s3.S3FileIO\n\
         debezium.sink.iceberg.s3.endpoint={s3_endpoint}\n\
         debezium.sink.iceberg.s3.path-style-access=true\n\
         debezium.sink.iceberg.s3.access-key-id={s3_access_key}\n\
         debezium.sink.iceberg.s3.secret-access-key={s3_secret_key}\n\
         debezium.sink.iceberg.client.region=us-east-1\n\
         debezium.sink.iceberg.upsert=true\n\
         debezium.sink.iceberg.upsert-keep-deletes=true\n\
         debezium.sink.iceberg.destination-uppercase-table-names=false\n\
         debezium.sink.iceberg.write.format.default=parquet\n\
         \n\
         debezium.source.connector.class=io.debezium.connector.postgresql.PostgresConnector\n\
         debezium.source.offset.storage=org.apache.kafka.connect.storage.FileOffsetBackingStore\n\
         debezium.source.offset.storage.file.filename=/debezium/data/{slug}-offsets.dat\n\
         debezium.source.offset.flush.interval.ms=0\n\
         debezium.source.database.hostname={hostname}\n\
         debezium.source.database.port={port}\n\
         debezium.source.database.user={user}\n\
         debezium.source.database.password={password}\n\
         debezium.source.database.dbname={dbname}\n\
         debezium.source.topic.prefix={slug}\n\
         debezium.source.schema.include.list={schema}\n\
         debezium.source.table.include.list={table}\n\
         debezium.source.plugin.name=pgoutput\n\
         debezium.source.slot.name={slug}_slot\n\
         debezium.source.publication.name={slug}_pub\n\
         debezium.source.publication.autocreate.mode=disabled\n\
         debezium.source.schema.history.internal=io.debezium.storage.file.history.FileSchemaHistory\n\
         debezium.source.schema.history.internal.file.filename=/debezium/data/{slug}-schema-history.dat\n",
        catalog_uri = sink.catalog_uri,
        warehouse = sink.warehouse,
        s3_endpoint = sink.s3_endpoint,
        s3_access_key = s3_access_key.expose_secret(),
        s3_secret_key = s3_secret_key.expose_secret(),
        hostname = source.database_hostname,
        port = source.database_port,
        user = source.database_user,
        password = database_password.expose_secret(),
        dbname = source.database_name,
        table = source.schema_qualified_table,
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeMap;

    use super::*;

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

    // ---- control-character rejection in the other interpolated fields ----

    #[test]
    fn source_spec_rejects_newline_in_hostname() {
        let err = DebeziumSourceSpec::new(
            ConnectorSlug::new("orders_pg").unwrap(),
            "pg.internal\nEXTRA=injected",
            5432,
            "oms",
            "cdc_reader",
            "public.orders",
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CdcSpecError::ControlCharacterInField {
                field: "database_hostname"
            }
        ));
    }

    #[test]
    fn source_spec_rejects_carriage_return_in_user() {
        let err = DebeziumSourceSpec::new(
            ConnectorSlug::new("orders_pg").unwrap(),
            "pg.internal",
            5432,
            "oms",
            "cdc_reader\r",
            "public.orders",
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CdcSpecError::ControlCharacterInField {
                field: "database_user"
            }
        ));
    }

    #[test]
    fn source_spec_rejects_newline_in_dbname_and_table() {
        assert!(
            DebeziumSourceSpec::new(
                ConnectorSlug::new("orders_pg").unwrap(),
                "pg.internal",
                5432,
                "oms\ninjected=1",
                "cdc_reader",
                "public.orders",
            )
            .is_err()
        );
        assert!(
            DebeziumSourceSpec::new(
                ConnectorSlug::new("orders_pg").unwrap(),
                "pg.internal",
                5432,
                "oms",
                "cdc_reader",
                "public.orders\ndebezium.source.table.include.list=.*",
            )
            .is_err()
        );
    }

    #[test]
    fn source_spec_accepts_clean_fields() {
        assert!(
            DebeziumSourceSpec::new(
                ConnectorSlug::new("orders_pg").unwrap(),
                "pg.internal",
                5432,
                "oms",
                "cdc_reader",
                "public.orders",
            )
            .is_ok()
        );
    }

    #[test]
    fn render_rejects_newline_in_password_without_leaking_it() {
        let source = DebeziumSourceSpec::new(
            ConnectorSlug::new("orders_pg").unwrap(),
            "pg.internal",
            5432,
            "oms",
            "cdc_reader",
            "public.orders",
        )
        .unwrap();
        let sink = IcebergSinkSpec {
            catalog_uri: "http://lakekeeper:8181/catalog".to_owned(),
            warehouse: "default".to_owned(),
            s3_endpoint: "http://rustfs:9000".to_owned(),
            catalog_token: None,
        };
        let malicious_password = SecretValue::new("hunter2\ndebezium.source.slot.name=evil_slot");
        let err = render_debezium_properties(
            &source,
            &sink,
            &malicious_password,
            &SecretValue::new("akid"),
            &SecretValue::new("secretkey"),
        )
        .unwrap_err();
        assert!(matches!(
            err,
            CdcSpecError::ControlCharacterInField {
                field: "database_password"
            }
        ));
        // The whole point: the error must never contain the password value.
        assert!(!format!("{err}").contains("hunter2"));
    }

    #[test]
    fn rendered_config_never_leaks_secrets_into_the_wrong_field() {
        let source = DebeziumSourceSpec::new(
            ConnectorSlug::new("orders_pg").unwrap(),
            "pg.internal",
            5432,
            "oms",
            "cdc_reader",
            "public.orders",
        )
        .unwrap();
        let sink = IcebergSinkSpec {
            catalog_uri: "http://lakekeeper:8181/catalog".to_owned(),
            warehouse: "default".to_owned(),
            s3_endpoint: "http://rustfs:9000".to_owned(),
            catalog_token: None,
        };
        let db_password = SecretValue::new("hunter2");
        let s3_key = SecretValue::new("akid");
        let s3_secret = SecretValue::new("secretkey");
        let rendered =
            render_debezium_properties(&source, &sink, &db_password, &s3_key, &s3_secret).unwrap();

        assert!(rendered.contains("debezium.source.database.password=hunter2"));
        assert!(rendered.contains("debezium.sink.iceberg.s3.access-key-id=akid"));
        assert!(rendered.contains("debezium.sink.iceberg.s3.secret-access-key=secretkey"));
        assert!(rendered.contains("debezium.source.slot.name=orders_pg_slot"));
        assert!(rendered.contains("debezium.source.publication.name=orders_pg_pub"));
        assert!(rendered.contains("debezium.source.table.include.list=public.orders"));
        assert!(rendered.contains("debezium.source.schema.include.list=public"));
        // File-based offset/schema-history storage, per P5-RESULT.md's
        // measured trap — never the Iceberg-backed default.
        assert!(rendered.contains("org.apache.kafka.connect.storage.FileOffsetBackingStore"));
        assert!(rendered.contains("io.debezium.storage.file.history.FileSchemaHistory"));
    }

    /// Parse a `.properties`-shaped body (`key=value` per line, blank lines
    /// and `#`-comments ignored) into a map, so the equivalence test below
    /// compares the PROPERTIES the two sources set rather than incidental
    /// text (line order, trailing whitespace, ...).
    fn parse_properties(body: &str) -> BTreeMap<String, String> {
        body.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (key.to_owned(), value.to_owned()))
            .collect()
    }

    /// Extract the demo connector's hand-written `application.properties`
    /// heredoc body straight out of the checked-in `docker-compose.yml`
    /// (`debezium-server`'s `command:`), and substitute the same literal
    /// values this test hands to [`render_debezium_properties`] for every
    /// `$${ENV_VAR}` the compose entrypoint shell would otherwise expand at
    /// container-start time — the compose file's own documented defaults
    /// (see `debezium-server`'s `environment:` block).
    fn demo_connector_compose_properties() -> String {
        const COMPOSE: &str = include_str!("../../../../docker-compose.yml");
        let start_marker = "cat > /debezium/config/application.properties <<EOF\n";
        let start = COMPOSE.find(start_marker).expect(
            "docker-compose.yml no longer contains the demo connector's heredoc start \
                     marker — has the debezium-server service been rewritten?",
        ) + start_marker.len();
        let rest = &COMPOSE[start..];
        let end = rest.find("\n        EOF\n").expect(
            "docker-compose.yml no longer contains the demo connector's heredoc end \
                     marker — has the debezium-server service been rewritten?",
        );
        rest[..end]
            .replace(
                "$${LAKEKEEPER_CATALOG_URI}",
                "http://lakekeeper:8181/catalog",
            )
            .replace("$${LAKEKEEPER_WAREHOUSE}", "default")
            .replace("$${LAKEKEEPER_TOKEN}", "test-lakekeeper-token")
            .replace("$${CH_RUSTFS_S3_ENDPOINT}", "http://rustfs:9000")
            .replace("$${RUSTFS_ACCESS_KEY}", "rustfsadmin")
            .replace("$${RUSTFS_SECRET_KEY}", "rustfsadmin")
            .replace("$${POSTGRES_USER}", "lakehouse")
            .replace("$${POSTGRES_PASSWORD}", "lakehouse")
            .replace("$${POSTGRES_DB}", "lakehouse")
    }

    /// The test that makes [`render_debezium_properties`] real (see its
    /// "Callers" doc section): render a spec built from the exact literal
    /// values the checked-in compose heredoc uses for the demo `p5cdc`
    /// connector, and assert the two produce the SAME set of properties.
    /// If a future change edits one without the other, this fails —
    /// that's the point.
    #[test]
    fn demo_connector_properties_match_the_checked_in_compose_file() {
        let source = DebeziumSourceSpec::new(
            ConnectorSlug::new("p5cdc").expect("p5cdc is a valid slug"),
            "postgres",
            5432,
            "lakehouse",
            "lakehouse",
            "p5_cdc.orders",
        )
        .expect("demo connector's literal fields are all control-character-free");
        let sink = IcebergSinkSpec {
            catalog_uri: "http://lakekeeper:8181/catalog".to_owned(),
            warehouse: "default".to_owned(),
            s3_endpoint: "http://rustfs:9000".to_owned(),
            catalog_token: Some(SecretValue::new("test-lakekeeper-token")),
        };
        let rendered = render_debezium_properties(
            &source,
            &sink,
            &SecretValue::new("lakehouse"),
            &SecretValue::new("rustfsadmin"),
            &SecretValue::new("rustfsadmin"),
        )
        .expect("the demo connector's literal fields never fail validation");

        let rendered_props = parse_properties(&rendered);
        // Sanity check that the parser actually found something — an empty
        // map on either side would make the equality assertion below
        // vacuously true and defeat the whole point of this test.
        assert!(!rendered_props.is_empty());

        let compose_body = demo_connector_compose_properties();
        let compose_props = parse_properties(&compose_body);
        assert!(!compose_props.is_empty());

        assert_eq!(
            rendered_props, compose_props,
            "render_debezium_properties has drifted from the checked-in \
             docker-compose.yml demo connector config — update whichever \
             one is now wrong"
        );
    }
}
