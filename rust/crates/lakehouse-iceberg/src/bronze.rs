//! Bronze table naming, schema conventions, and the default ingestion-day
//! partition spec — the mechanics side of ADR 0004
//! (`docs/adr/0004-bronze-naming-partitioning-retention.md`). Read that ADR
//! for the rationale; this module only carries the implementation.

use iceberg::spec::{NestedField, PrimitiveType, Schema, Transform, Type, UnboundPartitionSpec};
use iceberg::{Error, ErrorKind, NamespaceIdent, Result};

/// Every Bronze table lives in this single-level namespace.
///
/// One flat `bronze` namespace, not one namespace per tenant: tenant
/// isolation is a Lakekeeper *warehouse* boundary (ADR 0003), not a
/// namespace boundary — a warehouse is already scoped to one tenant, so
/// nesting tenant into the namespace path too would be a second, redundant
/// place to enforce the same isolation.
pub const BRONZE_NAMESPACE: &str = "bronze";

/// System column every Bronze table carries, recording when this crate
/// wrote the row (not any upstream/source event time — that stays whatever
/// column shape the source data already has).
///
/// Bronze's default partitioning is `day(_ingested_at)`, per the plan's
/// P1 acceptance criteria ("default: ingestion day"). Using a dedicated
/// system column, rather than partitioning on a source-provided timestamp,
/// means the partition scheme does not depend on that column existing, or
/// on it never being null, in every connector's data — a source event-time
/// partition can be added later as an *additional* spec without touching
/// this one.
pub const INGESTED_AT_COLUMN: &str = "_ingested_at";

/// Field id reserved for [`INGESTED_AT_COLUMN`]. Caller-supplied domain
/// columns must start numbering at [`FIRST_DOMAIN_FIELD_ID`] to leave this
/// id free.
pub const INGESTED_AT_FIELD_ID: i32 = 1;

/// First field id available to caller-supplied domain columns.
pub const FIRST_DOMAIN_FIELD_ID: i32 = 2;

/// Returns the [`NamespaceIdent`] for [`BRONZE_NAMESPACE`].
///
/// # Errors
///
/// Never actually fails (the namespace name is a compile-time constant),
/// but returns [`Result`] because `iceberg`'s `NamespaceIdent::from_strs`
/// does; a `#[must_use]` infallible wrapper would just move the `.unwrap()`
/// this crate refuses to write (workspace lint: `unwrap_used`) into every
/// caller instead.
pub fn bronze_namespace() -> Result<NamespaceIdent> {
    NamespaceIdent::from_strs([BRONZE_NAMESPACE])
}

/// Lowercases, and replaces every run of characters that are not
/// `[a-z0-9_]` with a single `_`, trimming leading/trailing underscores.
///
/// Matches the shape `lakehouse_api::tenant::BRONZE_CURATED` slugs already
/// use (`wisman-jakarta-per-bulan` style dataset slugs), so a connector
/// registry entry's slug can be handed to this function directly without a
/// separate naming pass.
///
/// # Errors
///
/// Returns an error if the result would be empty (e.g. the input was
/// entirely punctuation).
pub fn sanitize_table_name(raw: &str) -> Result<String> {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_underscore = false;
    for ch in raw.trim().chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            last_was_underscore = false;
        } else if !last_was_underscore && !out.is_empty() {
            out.push('_');
            last_was_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        return Err(Error::new(
            ErrorKind::DataInvalid,
            format!("{raw:?} sanitizes to an empty Bronze table name"),
        ));
    }
    Ok(out)
}

/// Builds the full Bronze schema: [`INGESTED_AT_COLUMN`] at
/// [`INGESTED_AT_FIELD_ID`], followed by `domain_fields` unchanged.
///
/// # Errors
///
/// Returns an error if `domain_fields` reuses [`INGESTED_AT_FIELD_ID`], or
/// if schema construction otherwise fails (e.g. a duplicate field id or
/// name among `domain_fields` themselves — validated by `iceberg`'s own
/// `SchemaBuilder`).
pub fn bronze_schema(domain_fields: Vec<NestedField>) -> Result<Schema> {
    if domain_fields.iter().any(|f| f.id == INGESTED_AT_FIELD_ID) {
        return Err(Error::new(
            ErrorKind::DataInvalid,
            format!(
                "domain field reuses reserved field id {INGESTED_AT_FIELD_ID} \
                 (that id is reserved for {INGESTED_AT_COLUMN}); domain fields \
                 must start at {FIRST_DOMAIN_FIELD_ID}"
            ),
        ));
    }
    let ingested_at = NestedField::required(
        INGESTED_AT_FIELD_ID,
        INGESTED_AT_COLUMN,
        Type::Primitive(PrimitiveType::Timestamp),
    );
    let mut fields = vec![std::sync::Arc::new(ingested_at)];
    fields.extend(domain_fields.into_iter().map(std::sync::Arc::new));
    Schema::builder()
        .with_schema_id(0)
        .with_fields(fields)
        .build()
}

/// The default Bronze partition spec: `day(_ingested_at)`.
///
/// # Errors
///
/// Returns an error if `schema` has no [`INGESTED_AT_COLUMN`] field (which
/// should not happen for a schema built via [`bronze_schema`]).
pub fn ingestion_day_partition_spec(schema: &Schema) -> Result<UnboundPartitionSpec> {
    UnboundPartitionSpec::builder()
        .add_partition_field(INGESTED_AT_FIELD_ID, "ingested_date", Transform::Day)
        .map(iceberg::spec::UnboundPartitionSpecBuilder::build)
        .map_err(|err| {
            Error::new(
                ErrorKind::DataInvalid,
                format!(
                    "failed to build the default day(_ingested_at) partition spec \
                     for schema {schema:?}: {err}"
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn sanitizes_dashes_to_underscores() {
        assert_eq!(
            sanitize_table_name("wisman-jakarta-per-bulan").unwrap(),
            "wisman_jakarta_per_bulan"
        );
    }

    #[test]
    fn sanitizes_mixed_case_and_spaces() {
        assert_eq!(
            sanitize_table_name("  My Table 01 ").unwrap(),
            "my_table_01"
        );
    }

    #[test]
    fn collapses_runs_of_punctuation() {
        assert_eq!(sanitize_table_name("a---b__c").unwrap(), "a_b_c");
    }

    #[test]
    fn rejects_all_punctuation_input() {
        assert!(sanitize_table_name("---").is_err());
    }

    #[test]
    fn rejects_empty_input() {
        assert!(sanitize_table_name("").is_err());
    }

    #[test]
    fn bronze_schema_reserves_ingested_at_field_id() {
        let domain = vec![NestedField::required(
            INGESTED_AT_FIELD_ID,
            "collides",
            Type::Primitive(PrimitiveType::String),
        )];
        assert!(bronze_schema(domain).is_err());
    }

    #[test]
    fn bronze_schema_includes_ingested_at_column() {
        let domain = vec![NestedField::required(
            FIRST_DOMAIN_FIELD_ID,
            "id",
            Type::Primitive(PrimitiveType::Long),
        )];
        let schema = bronze_schema(domain).unwrap();
        assert!(schema.field_by_name(INGESTED_AT_COLUMN).is_some());
        assert!(schema.field_by_name("id").is_some());
    }

    #[test]
    fn partition_spec_partitions_by_day_of_ingested_at() {
        let schema = bronze_schema(vec![NestedField::required(
            FIRST_DOMAIN_FIELD_ID,
            "id",
            Type::Primitive(PrimitiveType::Long),
        )])
        .unwrap();
        let spec = ingestion_day_partition_spec(&schema).unwrap();
        assert_eq!(spec.fields().len(), 1);
        assert_eq!(spec.fields()[0].transform, Transform::Day);
        assert_eq!(spec.fields()[0].source_id, INGESTED_AT_FIELD_ID);
    }

    #[test]
    fn bronze_namespace_is_flat_single_level() {
        let ns = bronze_namespace().unwrap();
        assert_eq!(ns.to_url_string(), BRONZE_NAMESPACE);
    }
}
