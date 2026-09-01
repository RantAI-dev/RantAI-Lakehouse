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
//! console "test connection + inspect schema" flow — is out of scope here,
//! matching `connectors::test_connection`'s existing "no real connectivity"
//! posture). [`render_debezium_properties`] is a pure string-template
//! function over already-resolved inputs; the caller resolves `secretRef`
//! via [`lakehouse_core::secret::SecretResolver`] (ADR 0002) before calling
//! it. Actually running a generated config (creating a new
//! `debezium-server` process/container per connector) is deployment
//! orchestration, not this crate's job — see ADR 0007's "what P5 does NOT
//! do" for why that stays out of scope this phase.

use lakehouse_core::secret::SecretValue;

/// One column of a source table, as a future schema-discovery step would
/// report it — `type_name` is whatever string the source system names the
/// column's type (e.g. Postgres's `information_schema.columns.data_type`,
/// or Debezium's own Kafka Connect schema type name).
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
/// Scalar types — including `json`/`jsonb`, which Debezium/dlt both
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

/// Everything [`render_debezium_properties`] needs to describe the
/// Postgres logical-replication source side of one connector.
#[derive(Debug, Clone)]
pub struct DebeziumSourceSpec {
    /// A short, unique name for this connector's Debezium topic prefix,
    /// replication slot, and publication — derived from the
    /// `connector.id` slug (ADR 0004's `sanitize_table_name` shape), never
    /// user-supplied verbatim, so it is always a safe identifier.
    pub connector_slug: String,
    /// Source Postgres host, resolved from `connector.host` — never
    /// logged by this function's caller per `connectors.rs`'s guarantee 2.
    pub database_hostname: String,
    /// Source Postgres port.
    pub database_port: u16,
    /// Source database name.
    pub database_name: String,
    /// Source database user.
    pub database_user: String,
    /// Schema-qualified table to capture, e.g. `public.orders`.
    pub schema_qualified_table: String,
}

/// Everything [`render_debezium_properties`] needs to describe the Bronze
/// Iceberg sink side, shared across every connector in one deployment
/// (Lakekeeper warehouse + RustFS/SeaweedFS endpoint, per ADR 0003/ADR
/// 0004) rather than per-connector.
#[derive(Debug, Clone)]
pub struct IcebergSinkSpec {
    /// Lakekeeper's REST catalog URI, e.g. `http://lakekeeper:8181/catalog`.
    pub catalog_uri: String,
    /// The tenant's Lakekeeper warehouse name (ADR 0003).
    pub warehouse: String,
    /// The S3-compatible endpoint backing that warehouse (`RustFS` or
    /// `SeaweedFS`, per ADR 0004/G2).
    pub s3_endpoint: String,
}

/// Render a `debezium-server-iceberg` `application.properties` body for one
/// connector — the control-plane -> runtime path ADR 0007 designs. Upsert
/// mode, initial snapshot then streaming, additive-only schema evolution
/// (Debezium/Iceberg's own default — see ADR 0006), file-based offset/
/// schema-history storage (P5-RESULT.md's measured trap: the
/// Iceberg-backed alternatives default ON and silently create two more
/// catalog tables that have nothing to do with Bronze).
///
/// Every secret value is interpolated into the returned `String` — by
/// construction, since a Debezium Server properties file has no
/// indirection mechanism for secrets. The caller is responsible for never
/// logging the returned string; this function itself never does (its
/// return value is not `Debug`-printed anywhere in this crate).
#[must_use]
pub fn render_debezium_properties(
    source: &DebeziumSourceSpec,
    sink: &IcebergSinkSpec,
    database_password: &SecretValue,
    s3_access_key: &SecretValue,
    s3_secret_key: &SecretValue,
) -> String {
    let slug = &source.connector_slug;
    format!(
        "debezium.sink.type=iceberg\n\
         debezium.sink.iceberg.catalog-name=lakekeeper\n\
         debezium.sink.iceberg.type=rest\n\
         debezium.sink.iceberg.uri={catalog_uri}\n\
         debezium.sink.iceberg.warehouse={warehouse}\n\
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
         debezium.source.table.include.list={table}\n\
         debezium.source.plugin.name=pgoutput\n\
         debezium.source.slot.name={slug}_slot\n\
         debezium.source.publication.name={slug}_pub\n\
         debezium.source.publication.autocreate.mode=filtered\n\
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
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

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

    #[test]
    fn rendered_config_never_leaks_secrets_into_the_wrong_field() {
        let source = DebeziumSourceSpec {
            connector_slug: "orders_pg".to_owned(),
            database_hostname: "pg.internal".to_owned(),
            database_port: 5432,
            database_name: "oms".to_owned(),
            database_user: "cdc_reader".to_owned(),
            schema_qualified_table: "public.orders".to_owned(),
        };
        let sink = IcebergSinkSpec {
            catalog_uri: "http://lakekeeper:8181/catalog".to_owned(),
            warehouse: "default".to_owned(),
            s3_endpoint: "http://rustfs:9000".to_owned(),
        };
        let db_password = SecretValue::new("hunter2");
        let s3_key = SecretValue::new("akid");
        let s3_secret = SecretValue::new("secretkey");
        let rendered =
            render_debezium_properties(&source, &sink, &db_password, &s3_key, &s3_secret);

        assert!(rendered.contains("debezium.source.database.password=hunter2"));
        assert!(rendered.contains("debezium.sink.iceberg.s3.access-key-id=akid"));
        assert!(rendered.contains("debezium.sink.iceberg.s3.secret-access-key=secretkey"));
        assert!(rendered.contains("debezium.source.slot.name=orders_pg_slot"));
        assert!(rendered.contains("debezium.source.publication.name=orders_pg_pub"));
        assert!(rendered.contains("debezium.source.table.include.list=public.orders"));
        // File-based offset/schema-history storage, per P5-RESULT.md's
        // measured trap — never the Iceberg-backed default.
        assert!(rendered.contains("org.apache.kafka.connect.storage.FileOffsetBackingStore"));
        assert!(rendered.contains("io.debezium.storage.file.history.FileSchemaHistory"));
    }
}
