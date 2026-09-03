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

/// System column every exported Gold table carries: when this crate wrote
/// the row (i.e. when the export run happened), not any business-date
/// column the mart itself may also carry.
pub const EXPORTED_AT_COLUMN: &str = "_exported_at";

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

    #[test]
    fn gold_schema_reserves_exported_at_field_id() {
        let domain = vec![NestedField::required(
            EXPORTED_AT_FIELD_ID,
            "collides",
            Type::Primitive(PrimitiveType::String),
        )];
        assert!(gold_schema(domain).is_err());
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
