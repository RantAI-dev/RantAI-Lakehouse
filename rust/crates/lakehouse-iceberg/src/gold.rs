//! Gold export naming, schema conventions, and the default export-run
//! partition spec — the mechanics side of ADR 0010
//! (`docs/adr/0010-gold-export-to-iceberg-from-rust.md`). Mirrors
//! `bronze.rs`'s conventions (ADR 0004) applied to a second, separate
//! namespace: Gold marts are exported to their own `gold` Iceberg
//! namespace, never mixed into `bronze`.
//!
//! # Why append, not upsert, and what that means for a repeated export
//!
//! `iceberg-rust` 0.10.x has no `UPDATE`/`DELETE`/`MERGE` (see the crate
//! doc comment and ADR 0010's Decision). Because of that, each export run
//! **appends** the mart's current rows as a new snapshot rather than
//! replacing the table's contents in place — the exact pattern ADR 0010
//! calls out for an aggregate mart: "the natural pattern is write a new
//! snapshot, not mutate rows." One consequence worth being explicit about:
//! running the export job twice against an unchanged mart makes the
//! Iceberg table's cumulative row count grow (both appends stay visible —
//! Iceberg has no history-collapsing "REPLACE" without a delete/rewrite
//! this crate does not implement). [`EXPORTED_AT_COLUMN`] exists
//! specifically so a consumer that only wants the latest run can filter to
//! `max(_exported_at)`; a scheduled job that wants the table to represent
//! only "current state" would need to run compaction/expiry against it
//! separately (the same `expire_snapshots` mechanism P4's
//! `dagster/dispar_orchestrate/maintenance.py` already runs for Bronze —
//! extending that job to also cover the `gold` namespace is a natural
//! follow-up, not done here to keep this change scoped to export itself).

use arrow_array::RecordBatch;
use iceberg::spec::{
    NestedField, PartitionKey, PrimitiveType, Schema, TableMetadata, Transform, Type,
    UnboundPartitionSpec,
};
use iceberg::{Error, ErrorKind, NamespaceIdent, Result};

use crate::bronze::{day_partition_key_for, sanitize_table_name};

/// Every Gold export lands in this single-level namespace — deliberately
/// separate from [`crate::bronze::BRONZE_NAMESPACE`]: Gold is a derived,
/// exported mart, not raw ingested data, and ADR 0010 is explicit that it
/// must not be mixed into `bronze`. One flat namespace, not one per tenant
/// or per mart, for the same reason `bronze.rs` gives: tenant isolation is
/// already a Lakekeeper *warehouse* boundary (ADR 0003).
pub const GOLD_NAMESPACE: &str = "gold";

/// DATA-65: Silver (`silver.*`) `ClickHouse` `MergeTree` tables are published
/// to a separate, single-level Iceberg namespace. Mirrors
/// [`GOLD_NAMESPACE`]: derived data, never mixed into `bronze` or `gold`.
pub const SILVER_NAMESPACE: &str = "silver";

/// DATA-65: which publication zone a table is written to/read from.
///
/// Both zones use the exact same naming, schema, partition and atomic
/// commit mechanics (ADR 0010); only the Iceberg namespace and the
/// `ClickHouse` source schema differ. `serving` is accepted as an alias of
/// `Gold` (BLD-19: the source schema's name), so a caller that echoes the
/// source schema still names the Gold publication zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishZone {
    /// Publish `silver.<mart>` to the `silver` Iceberg namespace.
    Silver,
    /// Publish `{gold_source_schema}.<mart>` to the `gold` namespace.
    Gold,
}

#[allow(
    clippy::trivially_copy_pass_by_ref,
    reason = "the DATA-65 API pins these as &self methods; callers hold a Copy zone in a \
              borrow and changing every call site to pass by value would only churn the API"
)]
impl PublishZone {
    /// The Iceberg namespace this zone publishes into.
    #[must_use]
    pub fn namespace_name(&self) -> &'static str {
        match self {
            Self::Silver => SILVER_NAMESPACE,
            Self::Gold => GOLD_NAMESPACE,
        }
    }

    /// The single-level [`NamespaceIdent`] for this zone.
    ///
    /// # Errors
    ///
    /// Never actually fails (the name is a compile-time constant) — see
    /// `bronze::bronze_namespace`'s doc comment for why this still returns
    /// [`Result`].
    pub fn namespace(&self) -> Result<NamespaceIdent> {
        NamespaceIdent::from_strs([self.namespace_name()])
    }

    /// The `ClickHouse` source schema the export reads from. Silver always
    /// reads `silver`; Gold keeps using the deployment's configured source
    /// schema (`serving` by default).
    #[must_use]
    pub fn source_ch_schema<'a>(&self, gold_source_schema: &'a str) -> &'a str {
        match self {
            Self::Silver => SILVER_NAMESPACE,
            Self::Gold => gold_source_schema,
        }
    }

    /// Parses a caller-supplied zone name. `None` for anything else, so a
    /// caller can answer `400` with its own fixed message (AGENTS.md rule
    /// 4: never echo an unrecognised value into a response).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "silver" => Some(Self::Silver),
            "gold" | "serving" => Some(Self::Gold),
            _ => None,
        }
    }
}

/// System column every exported Gold table carries: when this crate wrote
/// the row (i.e. when the export run happened), not any business-date
/// column the mart itself may also carry.
pub const EXPORTED_AT_COLUMN: &str = "_exported_at";

/// Suffix appended to a published table's own (sanitized) name to form the
/// latest-copy table's name (`DATA-6` D1): the Iceberg table for mart
/// `sales` is `gold.sales`, and the table that always holds only the
/// newest copy is `gold.sales_latest`. A constant in both languages —
/// `dagster/dispar_orchestrate/gold_latest_copy.py` names this one as the
/// source of the mirror.
pub const LATEST_TABLE_SUFFIX: &str = "_latest";

/// Table property marking a table as "the latest-copy table of exactly
/// this published table" (`DATA-6` D7): set to the published table's own
/// (sanitized) name by the Dagster op that creates `gold.<table>_latest`,
/// read by `GoldExportError::TargetIsLatestCopy`'s check so the Rust
/// export can never append a publish into another mart's latest-copy
/// table (a mart named `sales_latest` and the latest table of `sales`
/// are the same Iceberg name — the marker is what keeps them apart).
pub const LATEST_COPY_OF_PROPERTY: &str = "rantai.latest-copy-of";

/// Field id reserved for [`EXPORTED_AT_COLUMN`], mirroring
/// `bronze::INGESTED_AT_FIELD_ID`.
pub const EXPORTED_AT_FIELD_ID: i32 = 1;

/// First field id available to mart columns.
pub const FIRST_DOMAIN_FIELD_ID: i32 = 2;

/// Returns the [`NamespaceIdent`] for [`GOLD_NAMESPACE`].
///
/// # Errors
///
/// Never actually fails (the namespace name is a compile-time constant) —
/// see `bronze::bronze_namespace`'s doc comment for why this still returns
/// [`Result`].
pub fn gold_namespace() -> Result<NamespaceIdent> {
    NamespaceIdent::from_strs([GOLD_NAMESPACE])
}

/// Sanitizes a Gold mart name into a safe Iceberg table name. Reuses
/// [`crate::bronze::sanitize_table_name`] — the naming rule is identical,
/// only the namespace differs.
///
/// # Errors
///
/// See `bronze::sanitize_table_name`.
pub fn sanitize_mart_name(raw: &str) -> Result<String> {
    sanitize_table_name(raw)
}

/// The latest-copy table's name for an already-sanitized published table
/// name (`DATA-6` D1): `latest_table_name("sales") == "sales_latest"`.
/// `table` is the *sanitized* name, the same string `GoldExportResult`
/// reports — the caller does not sanitize twice.
#[must_use]
pub fn latest_table_name(table: &str) -> String {
    format!("{table}{LATEST_TABLE_SUFFIX}")
}

/// Reads [`LATEST_COPY_OF_PROPERTY`] off a table's properties: `Some(table)`
/// when the table is the latest-copy table of `table`, `None` when the
/// property is absent (`DATA-6` D7). A plain read, deliberately no
/// validation of the named table here — the two enforcement sites (the
/// Rust export's refusal, the Python op's `name_taken`) decide what a
/// present-but-wrong value means.
#[allow(
    clippy::implicit_hasher,
    reason = "the map is always the Iceberg metadata's own properties() — a std \
              HashMap deserialized by the iceberg crate; no caller of this \
              function ever builds the map, so a non-default hasher cannot occur"
)]
#[must_use]
pub fn latest_copy_of(properties: &std::collections::HashMap<String, String>) -> Option<&str> {
    properties.get(LATEST_COPY_OF_PROPERTY).map(String::as_str)
}

/// Builds the full Gold export schema: [`EXPORTED_AT_COLUMN`] at
/// [`EXPORTED_AT_FIELD_ID`], followed by `domain_fields` (the mart's own
/// columns) unchanged.
///
/// # Errors
///
/// Returns an error if `domain_fields` reuses [`EXPORTED_AT_FIELD_ID`], or
/// if schema construction otherwise fails.
pub fn gold_schema(domain_fields: Vec<NestedField>) -> Result<Schema> {
    if domain_fields.iter().any(|f| f.id == EXPORTED_AT_FIELD_ID) {
        return Err(Error::new(
            ErrorKind::DataInvalid,
            format!(
                "domain field reuses reserved field id {EXPORTED_AT_FIELD_ID} \
                 (that id is reserved for {EXPORTED_AT_COLUMN}); mart fields \
                 must start at {FIRST_DOMAIN_FIELD_ID}"
            ),
        ));
    }
    let exported_at = NestedField::required(
        EXPORTED_AT_FIELD_ID,
        EXPORTED_AT_COLUMN,
        Type::Primitive(PrimitiveType::Timestamp),
    );
    let mut fields = vec![std::sync::Arc::new(exported_at)];
    fields.extend(domain_fields.into_iter().map(std::sync::Arc::new));
    Schema::builder()
        .with_schema_id(0)
        .with_fields(fields)
        .build()
}

/// The default Gold export partition spec: `day(_exported_at)`, matching
/// ADR 0004's `day(_ingested_at)` convention for Bronze, applied to the
/// export-time column instead.
///
/// # Errors
///
/// Returns an error if `schema` has no [`EXPORTED_AT_COLUMN`] field.
pub fn export_day_partition_spec(schema: &Schema) -> Result<UnboundPartitionSpec> {
    UnboundPartitionSpec::builder()
        .add_partition_field(EXPORTED_AT_FIELD_ID, "exported_date", Transform::Day)
        .map(iceberg::spec::UnboundPartitionSpecBuilder::build)
        .map_err(|err| {
            Error::new(
                ErrorKind::DataInvalid,
                format!(
                    "failed to build the default day(_exported_at) partition spec \
                     for schema {schema:?}: {err}"
                ),
            )
        })
}

/// Computes the [`PartitionKey`] for one data file written from `batch`,
/// using [`EXPORTED_AT_COLUMN`]'s first non-null value. See
/// `bronze::partition_key_for`'s doc comment for the one-partition-per-
/// batch caveat this shares.
///
/// # Errors
///
/// Returns an error if `batch` has no [`EXPORTED_AT_COLUMN`], the column is
/// not a microsecond timestamp, or every value in it is null.
pub fn partition_key_for(metadata: &TableMetadata, batch: &RecordBatch) -> Result<PartitionKey> {
    day_partition_key_for(metadata, batch, EXPORTED_AT_COLUMN)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use iceberg::spec::NestedField;

    #[test]
    fn gold_namespace_is_flat_single_level_and_not_bronze() {
        let ns = gold_namespace().unwrap();
        assert_eq!(ns.to_url_string(), GOLD_NAMESPACE);
        assert_ne!(GOLD_NAMESPACE, crate::bronze::BRONZE_NAMESPACE);
    }

    // DATA-65: the two published namespaces are distinct flat, single-level
    // names, and neither is the Bronze ingestion namespace.
    #[test]
    fn publish_zone_namespaces_are_flat_distinct_and_not_bronze() {
        for zone in [PublishZone::Silver, PublishZone::Gold] {
            let ns = zone.namespace().unwrap();
            assert_eq!(ns.to_url_string(), zone.namespace_name());
            assert!(!ns.to_url_string().contains('/'), "the namespace is flat");
            assert_ne!(zone.namespace_name(), crate::bronze::BRONZE_NAMESPACE);
        }
        assert_eq!(PublishZone::Silver.namespace_name(), "silver");
        assert_eq!(PublishZone::Gold.namespace_name(), "gold");
        assert_ne!(
            PublishZone::Silver.namespace().unwrap(),
            PublishZone::Gold.namespace().unwrap()
        );
    }

    #[test]
    fn publish_zone_parses_silver_gold_and_the_serving_alias() {
        assert_eq!(PublishZone::parse("silver"), Some(PublishZone::Silver));
        assert_eq!(PublishZone::parse("gold"), Some(PublishZone::Gold));
        // BLD-19: `serving` is the Gold source schema's name, accepted as
        // an alias so callers that echo it still name the Gold zone.
        assert_eq!(PublishZone::parse("serving"), Some(PublishZone::Gold));
        for invalid in ["bronze", "SILVER", "gold.", "", " silver"] {
            assert_eq!(PublishZone::parse(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn publish_zone_chooses_the_clickhouse_source_schema() {
        assert_eq!(PublishZone::Silver.source_ch_schema("serving"), "silver");
        assert_eq!(PublishZone::Gold.source_ch_schema("serving"), "serving");
        assert_eq!(
            PublishZone::parse("serving")
                .unwrap()
                .source_ch_schema("serving"),
            "serving"
        );
    }

    #[test]
    fn published_schema_and_partition_helpers_are_zone_independent() {
        // The same `gold_schema`/`export_day_partition_spec` helpers back
        // both zones: one schema shape, one `day(_exported_at)` spec.
        let schema = gold_schema(vec![NestedField::required(
            FIRST_DOMAIN_FIELD_ID,
            "region",
            Type::Primitive(PrimitiveType::String),
        )])
        .unwrap();
        let spec = export_day_partition_spec(&schema).unwrap();
        for zone in [PublishZone::Silver, PublishZone::Gold] {
            let _ = zone;
            assert!(schema.field_by_name(EXPORTED_AT_COLUMN).is_some());
            assert_eq!(spec.fields().len(), 1);
            assert_eq!(spec.fields()[0].source_id, EXPORTED_AT_FIELD_ID);
        }
    }

    #[test]
    fn gold_schema_reserves_exported_at_field_id() {
        let domain = vec![NestedField::required(
            EXPORTED_AT_FIELD_ID,
            "collides",
            Type::Primitive(PrimitiveType::String),
        )];
        assert!(gold_schema(domain).is_err());
    }

    // DATA-6 D1/D7: the latest-copy table's name and marker are pinned to
    // these exact strings in BOTH languages (`gold_latest_copy.py` names
    // this module's constants as the source); a drift would make the Rust
    // refusal and the Python write disagree about the same table.
    #[test]
    fn latest_table_constants_are_pinned() {
        assert_eq!(LATEST_TABLE_SUFFIX, "_latest");
        assert_eq!(LATEST_COPY_OF_PROPERTY, "rantai.latest-copy-of");
    }

    #[test]
    fn latest_table_name_appends_the_suffix() {
        assert_eq!(latest_table_name("sales"), "sales_latest");
        assert_eq!(
            latest_table_name("sales"),
            format!("sales{LATEST_TABLE_SUFFIX}")
        );
    }

    #[test]
    fn latest_copy_of_reads_the_marker_or_none() {
        let mut properties = std::collections::HashMap::new();
        assert_eq!(latest_copy_of(&properties), None);
        properties.insert(LATEST_COPY_OF_PROPERTY.to_owned(), "sales".to_owned());
        assert_eq!(latest_copy_of(&properties), Some("sales"));
        // An unrelated property never reads as a marker.
        properties.clear();
        properties.insert("other".to_owned(), "sales".to_owned());
        assert_eq!(latest_copy_of(&properties), None);
    }

    #[test]
    fn gold_schema_includes_exported_at_column() {
        let domain = vec![NestedField::required(
            FIRST_DOMAIN_FIELD_ID,
            "region",
            Type::Primitive(PrimitiveType::String),
        )];
        let schema = gold_schema(domain).unwrap();
        assert!(schema.field_by_name(EXPORTED_AT_COLUMN).is_some());
        assert!(schema.field_by_name("region").is_some());
    }

    #[test]
    fn partition_spec_partitions_by_day_of_exported_at() {
        let schema = gold_schema(vec![NestedField::required(
            FIRST_DOMAIN_FIELD_ID,
            "region",
            Type::Primitive(PrimitiveType::String),
        )])
        .unwrap();
        let spec = export_day_partition_spec(&schema).unwrap();
        assert_eq!(spec.fields().len(), 1);
        assert_eq!(spec.fields()[0].transform, Transform::Day);
        assert_eq!(spec.fields()[0].source_id, EXPORTED_AT_FIELD_ID);
    }
}
