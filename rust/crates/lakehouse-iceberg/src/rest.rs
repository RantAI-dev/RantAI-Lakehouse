//! Read-only Lakekeeper Iceberg REST surface for `/api/lakehouse/*`.
//!
//! Namespace/table listing and `load_table` go through `iceberg` 0.10.1's
//! typed `Catalog` trait — the SAME `RestCatalog` connection `IcebergClient`
//! authenticates via `catalog.rs`'s vended-credentials header and
//! `catalog_token` — not a second raw-HTTP client.
//!
//! [`list_warehouses`] does NOT call Lakekeeper's Management API
//! (`GET /management/v1/warehouse`): that endpoint is admin-scoped
//! (confirmed: `docker-compose.yml`'s `lakekeeper-warehouse-init` is the
//! only service holding `admin.jwt`, and `maintenance.py`'s module doc
//! reaches the same conclusion independently) and no long-running service
//! in this stack holds an admin token, so it reports the single configured
//! warehouse, verified reachable via a `list_namespaces` call with this
//! client's own credential.
//!
//! This module stays free of any HTTP response shape (`serde_json::json!`
//! belongs to `lakehouse-api`'s `routes::lakehouse`, which maps these types
//! into the `/api/lakehouse/*` contract).
//!
//! # `NotFound` mapping
//!
//! `iceberg-catalog-rest` 0.10.1 maps an HTTP 404 to
//! `iceberg::ErrorKind::TableNotFound` (`load_table`) or
//! `iceberg::ErrorKind::NamespaceNotFound` (`list_tables`). This module
//! checks `iceberg::Error::kind()` on the raw error BEFORE it is converted
//! to [`IcebergError`] (whose `From<iceberg::Error>` impl only keeps the
//! `Display` string — `catalog.rs:90-94` — and loses the kind), so
//! `RestError::NotFound` is reachable from real 404s, not left as dead code
//! deferred to a later task.

use std::collections::HashMap;

use iceberg::spec::Summary;
use iceberg::{ErrorKind, NamespaceIdent, TableIdent};
use thiserror::Error;

use crate::catalog::{IcebergClient, IcebergError};

/// Errors from the read-only Lakekeeper surface.
#[derive(Debug, Error)]
pub enum RestError {
    /// The underlying catalog call failed for a reason other than
    /// "not found". Its `Display` text carries upstream detail and is
    /// classified into a fixed message before it reaches an HTTP response
    /// — see `lakehouse-api`'s `routes::lakehouse::classify_rest_error`.
    #[error("iceberg catalog read failed: {0}")]
    Catalog(#[from] IcebergError),
    /// The requested namespace or table does not exist.
    #[error("not found")]
    NotFound,
}

/// Maps a raw `iceberg::Error` to [`RestError`], reading `err.kind()`
/// before the kind is lost to `IcebergError::from`'s `Display`-only
/// conversion. `not_found_kind` is the `ErrorKind` this call site's
/// underlying REST endpoint maps its own 404 to (`TableNotFound` for
/// `load_table`, `NamespaceNotFound` for `list_tables`).
fn map_catalog_error(err: iceberg::Error, not_found_kind: ErrorKind) -> RestError {
    if err.kind() == not_found_kind {
        RestError::NotFound
    } else {
        RestError::Catalog(IcebergError::from(err))
    }
}

/// One Lakekeeper warehouse this service is configured against, with a
/// liveness check.
#[derive(Debug)]
pub struct WarehouseSummary {
    /// The configured Lakekeeper warehouse identifier
    /// (`IcebergClientConfig::warehouse`).
    pub name: String,
    /// Whether a `list_namespaces` call against this warehouse, using this
    /// service's own credential, succeeded. A 401 or 403 (an expired or
    /// insufficiently scoped credential) also reads as `false` — this is
    /// not a claim that the warehouse itself is offline, only that this
    /// credential could not currently reach it.
    pub reachable: bool,
}

/// Reports the single Lakekeeper warehouse this service is configured
/// against (see the module doc — the Management API's warehouse list is
/// admin-scoped and unavailable here), verified reachable with a
/// `list_namespaces` call. Never fails: an unreachable warehouse is
/// reported as `reachable: false`, not an error.
#[must_use]
pub async fn list_warehouses(
    client: &IcebergClient,
    warehouse_name: &str,
) -> Vec<WarehouseSummary> {
    let reachable = client.as_catalog().list_namespaces(None).await.is_ok();
    vec![WarehouseSummary {
        name: warehouse_name.to_owned(),
        reachable,
    }]
}

/// Lists top-level namespaces in the configured warehouse.
///
/// # Errors
/// Returns [`RestError::NotFound`] if the root namespace does not exist,
/// [`RestError::Catalog`] for any other catalog failure.
pub async fn list_namespaces(client: &IcebergClient) -> Result<Vec<NamespaceIdent>, RestError> {
    client
        .as_catalog()
        .list_namespaces(None)
        .await
        .map_err(|err| map_catalog_error(err, ErrorKind::NamespaceNotFound))
}

/// Lists table identifiers in `namespace` — a single, cheap listing call,
/// deliberately NOT a per-table `load_table` loop, so the caller can bound
/// per-table loads.
///
/// # Errors
/// Returns [`RestError::NotFound`] if `namespace` does not exist,
/// [`RestError::Catalog`] for any other catalog failure.
pub async fn list_table_idents(
    client: &IcebergClient,
    namespace: &NamespaceIdent,
) -> Result<Vec<TableIdent>, RestError> {
    client
        .as_catalog()
        .list_tables(namespace)
        .await
        .map_err(|err| map_catalog_error(err, ErrorKind::NamespaceNotFound))
}

/// File/record/byte totals for a table's current (or a specific) snapshot.
#[derive(Debug, Default, Clone, Copy)]
pub struct TableStats {
    /// `total-data-files` from the snapshot summary, if present.
    pub file_count: Option<u64>,
    /// `total-records` from the snapshot summary, if present.
    pub record_count: Option<u64>,
    /// `total-files-size` from the snapshot summary, if present.
    pub total_bytes: Option<u64>,
}

/// File/record/byte totals for a table's current snapshot — read from
/// `iceberg-rust`'s own snapshot-summary totals, which its
/// `fast_append`/`Transaction` path populates (verified in
/// `iceberg-0.10.1/src/spec/snapshot_summary.rs`). A table committed by a
/// DIFFERENT writer (Debezium's Iceberg sink, dlt) may not populate the
/// same keys — this returns `None` per field rather than assuming every
/// writer agrees. Whether tables written by Debezium or dlt populate these
/// keys is not yet verified.
#[must_use]
pub fn stats_from_summary(summary: &Summary) -> TableStats {
    TableStats {
        file_count: parse_u64(summary, "total-data-files"),
        record_count: parse_u64(summary, "total-records"),
        total_bytes: parse_u64(summary, "total-files-size"),
    }
}

fn parse_u64(summary: &Summary, key: &str) -> Option<u64> {
    summary.additional_properties.get(key)?.parse().ok()
}

/// One table row for a namespace listing.
#[derive(Debug)]
pub struct TableSummary {
    /// The table's namespace, as a URL-joined string (e.g. `"bronze"`).
    pub namespace: String,
    /// The table's name within its namespace.
    pub name: String,
    /// The table's Iceberg format version (1, 2 or 3).
    pub format_version: i32,
    /// The table's current snapshot id, `None` for a table with no
    /// snapshots yet.
    pub current_snapshot_id: Option<i64>,
    /// The current snapshot's commit timestamp, `None` with no snapshots.
    pub last_updated_ms: Option<i64>,
    /// Totals from the current snapshot's summary.
    pub stats: TableStats,
}

/// Loads one table's summary — namespace, name, format version, current
/// snapshot id/timestamp and stats.
///
/// # Errors
/// Returns [`RestError::NotFound`] if the table does not exist,
/// [`RestError::Catalog`] for any other catalog failure.
pub async fn load_table_summary(
    client: &IcebergClient,
    ident: &TableIdent,
) -> Result<TableSummary, RestError> {
    let table = client
        .as_catalog()
        .load_table(ident)
        .await
        .map_err(|err| map_catalog_error(err, ErrorKind::TableNotFound))?;
    let metadata = table.metadata();
    let snapshot = metadata.current_snapshot();
    Ok(TableSummary {
        namespace: ident.namespace().to_url_string(),
        name: ident.name().to_owned(),
        format_version: metadata.format_version() as i32,
        current_snapshot_id: snapshot.map(|s| s.snapshot_id()),
        last_updated_ms: snapshot.map(|s| s.timestamp_ms()),
        stats: snapshot
            .map(|s| stats_from_summary(s.summary()))
            .unwrap_or_default(),
    })
}

/// One column of a table's current schema.
#[derive(Debug)]
pub struct FieldDetail {
    /// The field's schema id, unique within the table's schema.
    pub id: i32,
    /// The field's name.
    pub name: String,
    /// The field's Iceberg type, rendered as its `Display` string (e.g.
    /// `"long"`, `"struct<...>"`).
    pub r#type: String,
    /// Whether the field is required (non-nullable).
    pub required: bool,
}

/// One field of the table's default partition spec.
#[derive(Debug)]
pub struct PartitionFieldDetail {
    /// The source column id this partition field transforms.
    pub source_id: i32,
    /// The transform applied to the source column, rendered as its
    /// `Display` string (e.g. `"identity"`, `"day"`, `"bucket[16]"`).
    pub transform: String,
    /// The partition field's name.
    pub name: String,
}

/// One snapshot in a table's history.
#[derive(Debug)]
pub struct SnapshotDetail {
    /// The snapshot id.
    pub id: i64,
    /// The parent snapshot id, `None` for the table's first snapshot.
    pub parent_id: Option<i64>,
    /// The snapshot's commit timestamp, in epoch milliseconds.
    pub timestamp_ms: i64,
    /// The snapshot's operation (`"append"`, `"replace"`, `"overwrite"` or
    /// `"delete"`).
    pub operation: String,
    /// `added-records` from the snapshot summary, if present.
    pub added_records: Option<u64>,
    /// `deleted-records` from the snapshot summary, if present.
    pub deleted_records: Option<u64>,
    /// `total-records` from the snapshot summary, if present.
    pub total_records: Option<u64>,
    /// `total-data-files` from the snapshot summary, if present.
    pub total_data_files: Option<u64>,
    /// The same totals [`stats_from_summary`] reads for the current
    /// snapshot, kept here per-snapshot since `total_bytes` has no
    /// dedicated field above.
    pub stats: TableStats,
}

/// Full detail for one table: schema, partition spec, properties, snapshot
/// history and stats.
#[derive(Debug)]
pub struct TableDetail {
    /// The table's current schema, one row per column.
    pub schema: Vec<FieldDetail>,
    /// The table's default partition spec fields.
    pub partition_fields: Vec<PartitionFieldDetail>,
    /// The table's Iceberg properties.
    pub properties: HashMap<String, String>,
    /// The table's full snapshot history, sorted oldest first by
    /// `timestamp_ms` ascending (ties broken by `id` ascending). `iceberg`
    /// 0.10.1's `TableMetadata::snapshots()` iterates a map keyed by
    /// snapshot id, not commit order, so [`load_table_detail`] sorts the
    /// collected vector explicitly rather than trusting iteration order.
    pub snapshots: Vec<SnapshotDetail>,
    /// `snapshots.len()` — kept as a separate field so a caller does not
    /// need to materialize the full snapshot list just to know its size.
    pub snapshot_count: usize,
    /// The table's metadata-log length (how many prior metadata files this
    /// table has had).
    pub metadata_log_count: usize,
    /// Totals from the current snapshot's summary.
    pub stats: TableStats,
}

/// Loads full detail for one table.
///
/// # Errors
/// Returns [`RestError::NotFound`] if the table does not exist,
/// [`RestError::Catalog`] for any other catalog failure.
pub async fn load_table_detail(
    client: &IcebergClient,
    ident: &TableIdent,
) -> Result<TableDetail, RestError> {
    let table = client
        .as_catalog()
        .load_table(ident)
        .await
        .map_err(|err| map_catalog_error(err, ErrorKind::TableNotFound))?;
    let metadata = table.metadata();
    Ok(TableDetail {
        schema: metadata
            .current_schema()
            .as_struct()
            .fields()
            .iter()
            .map(|f| FieldDetail {
                id: f.id,
                name: f.name.clone(),
                r#type: f.field_type.to_string(),
                required: f.required,
            })
            .collect(),
        partition_fields: metadata
            .default_partition_spec()
            .fields()
            .iter()
            .map(|f| PartitionFieldDetail {
                source_id: f.source_id,
                transform: f.transform.to_string(),
                name: f.name.clone(),
            })
            .collect(),
        properties: metadata.properties().clone(),
        snapshots: {
            let mut snapshots: Vec<SnapshotDetail> = metadata
                .snapshots()
                .map(|s| SnapshotDetail {
                    id: s.snapshot_id(),
                    parent_id: s.parent_snapshot_id(),
                    timestamp_ms: s.timestamp_ms(),
                    operation: s.summary().operation.as_str().to_owned(),
                    added_records: parse_u64(s.summary(), "added-records"),
                    deleted_records: parse_u64(s.summary(), "deleted-records"),
                    total_records: parse_u64(s.summary(), "total-records"),
                    total_data_files: parse_u64(s.summary(), "total-data-files"),
                    stats: stats_from_summary(s.summary()),
                })
                .collect();
            // `TableMetadata::snapshots()` iterates a map keyed by snapshot
            // id, not commit order — sort explicitly so `snapshots` is
            // actually oldest-first, as documented on the field.
            snapshots.sort_by(|a, b| a.timestamp_ms.cmp(&b.timestamp_ms).then(a.id.cmp(&b.id)));
            snapshots
        },
        snapshot_count: metadata.snapshots().len(),
        metadata_log_count: metadata.metadata_log().len(),
        stats: metadata
            .current_snapshot()
            .map(|s| stats_from_summary(s.summary()))
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use iceberg::spec::Operation;
    use std::collections::HashMap;

    use super::*;

    fn summary_with(pairs: &[(&str, &str)]) -> Summary {
        let mut additional_properties = HashMap::new();
        for (k, v) in pairs {
            additional_properties.insert((*k).to_owned(), (*v).to_owned());
        }
        Summary {
            operation: Operation::Append,
            additional_properties,
        }
    }

    #[test]
    fn table_stats_reads_totals_from_snapshot_summary() {
        let summary = summary_with(&[
            ("total-data-files", "7"),
            ("total-records", "1200"),
            ("total-files-size", "451200"),
        ]);

        let stats = stats_from_summary(&summary);

        assert_eq!(stats.file_count, Some(7));
        assert_eq!(stats.record_count, Some(1200));
        assert_eq!(stats.total_bytes, Some(451_200));
    }

    #[test]
    fn table_stats_nulls_out_when_summary_lacks_the_keys() {
        let summary = summary_with(&[]);
        let stats = stats_from_summary(&summary);
        assert_eq!(stats.file_count, None);
        assert_eq!(stats.record_count, None);
        assert_eq!(stats.total_bytes, None);
    }

    #[test]
    fn snapshot_summary_counts_are_parsed_when_present() {
        let summary = summary_with(&[
            ("added-records", "50"),
            ("deleted-records", "5"),
            ("total-records", "150"),
            ("total-data-files", "2"),
        ]);

        assert_eq!(parse_u64(&summary, "added-records"), Some(50));
        assert_eq!(parse_u64(&summary, "deleted-records"), Some(5));
        assert_eq!(parse_u64(&summary, "total-records"), Some(150));
        assert_eq!(parse_u64(&summary, "total-data-files"), Some(2));
    }

    #[test]
    fn snapshot_summary_counts_are_none_when_absent_or_non_numeric() {
        let absent = summary_with(&[]);
        assert_eq!(parse_u64(&absent, "added-records"), None);

        let non_numeric = summary_with(&[("added-records", "not-a-number")]);
        assert_eq!(parse_u64(&non_numeric, "added-records"), None);
    }

    mod wiremock_tests {
        use iceberg::{NamespaceIdent, TableIdent};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use crate::catalog::{IcebergClient, IcebergClientConfig};

        /// A minimal, spec-valid Iceberg REST `LoadTableResult` JSON
        /// fixture, hand-written (not captured live): format version 2,
        /// one schema, a default partition spec with one field, two
        /// snapshots with `total-*`/`added-records` summary keys, and a
        /// one-entry metadata log.
        const LOAD_TABLE_FIXTURE: &str = r#"{
            "metadata-location": "s3://bucket/warehouse/bronze/orders/metadata/00001.metadata.json",
            "metadata": {
                "format-version": 2,
                "table-uuid": "9c12d441-03fe-4bb1-8e5c-b2e7c1a04a3d",
                "location": "s3://bucket/warehouse/bronze/orders",
                "last-sequence-number": 2,
                "last-updated-ms": 1700000000000,
                "last-column-id": 2,
                "schemas": [
                    {
                        "schema-id": 0,
                        "type": "struct",
                        "fields": [
                            {"id": 1, "name": "id", "required": true, "type": "long"},
                            {"id": 2, "name": "created_at", "required": false, "type": "timestamp"}
                        ]
                    }
                ],
                "current-schema-id": 0,
                "partition-specs": [
                    {
                        "spec-id": 0,
                        "fields": [
                            {"source-id": 2, "field-id": 1000, "name": "created_at_day", "transform": "day"}
                        ]
                    }
                ],
                "default-spec-id": 0,
                "last-partition-id": 1000,
                "properties": {"write.format.default": "parquet"},
                "current-snapshot-id": 2,
                "snapshots": [
                    {
                        "snapshot-id": 1,
                        "sequence-number": 1,
                        "timestamp-ms": 1699999000000,
                        "manifest-list": "s3://bucket/warehouse/bronze/orders/metadata/snap-1.avro",
                        "summary": {
                            "operation": "append",
                            "added-records": "100",
                            "total-records": "100",
                            "total-data-files": "1",
                            "total-files-size": "1024"
                        }
                    },
                    {
                        "snapshot-id": 2,
                        "parent-snapshot-id": 1,
                        "sequence-number": 2,
                        "timestamp-ms": 1700000000000,
                        "manifest-list": "s3://bucket/warehouse/bronze/orders/metadata/snap-2.avro",
                        "summary": {
                            "operation": "append",
                            "added-records": "50",
                            "deleted-records": "0",
                            "total-records": "150",
                            "total-data-files": "2",
                            "total-files-size": "2048"
                        }
                    }
                ],
                "metadata-log": [
                    {"timestamp-ms": 1699999000000, "metadata-file": "s3://bucket/warehouse/bronze/orders/metadata/00000.metadata.json"}
                ],
                "sort-orders": [{"order-id": 0, "fields": []}],
                "default-sort-order-id": 0
            }
        }"#;

        /// A second `LoadTableResult` fixture whose snapshot ids run
        /// AGAINST commit order (the newer snapshot has the smaller id),
        /// so a sort keyed on id alone would misorder it. Otherwise the
        /// same shape as [`LOAD_TABLE_FIXTURE`].
        const LOAD_TABLE_FIXTURE_UNORDERED_IDS: &str = r#"{
            "metadata-location": "s3://bucket/warehouse/bronze/orders/metadata/00001.metadata.json",
            "metadata": {
                "format-version": 2,
                "table-uuid": "9c12d441-03fe-4bb1-8e5c-b2e7c1a04a3d",
                "location": "s3://bucket/warehouse/bronze/orders",
                "last-sequence-number": 2,
                "last-updated-ms": 1700000000000,
                "last-column-id": 2,
                "schemas": [
                    {
                        "schema-id": 0,
                        "type": "struct",
                        "fields": [
                            {"id": 1, "name": "id", "required": true, "type": "long"},
                            {"id": 2, "name": "created_at", "required": false, "type": "timestamp"}
                        ]
                    }
                ],
                "current-schema-id": 0,
                "partition-specs": [
                    {
                        "spec-id": 0,
                        "fields": [
                            {"source-id": 2, "field-id": 1000, "name": "created_at_day", "transform": "day"}
                        ]
                    }
                ],
                "default-spec-id": 0,
                "last-partition-id": 1000,
                "properties": {"write.format.default": "parquet"},
                "current-snapshot-id": 10,
                "snapshots": [
                    {
                        "snapshot-id": 20,
                        "sequence-number": 1,
                        "timestamp-ms": 1699999000000,
                        "manifest-list": "s3://bucket/warehouse/bronze/orders/metadata/snap-20.avro",
                        "summary": {
                            "operation": "append",
                            "added-records": "100",
                            "total-records": "100",
                            "total-data-files": "1",
                            "total-files-size": "1024"
                        }
                    },
                    {
                        "snapshot-id": 10,
                        "parent-snapshot-id": 20,
                        "sequence-number": 2,
                        "timestamp-ms": 1700000000000,
                        "manifest-list": "s3://bucket/warehouse/bronze/orders/metadata/snap-10.avro",
                        "summary": {
                            "operation": "append",
                            "added-records": "50",
                            "deleted-records": "0",
                            "total-records": "150",
                            "total-data-files": "2",
                            "total-files-size": "2048"
                        }
                    }
                ],
                "metadata-log": [
                    {"timestamp-ms": 1699999000000, "metadata-file": "s3://bucket/warehouse/bronze/orders/metadata/00000.metadata.json"}
                ],
                "sort-orders": [{"order-id": 0, "fields": []}],
                "default-sort-order-id": 0
            }
        }"#;

        async fn connect(server: &MockServer) -> IcebergClient {
            Mock::given(method("GET"))
                .and(path("/v1/config"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
                )
                .mount(server)
                .await;

            let config = IcebergClientConfig {
                catalog_uri: server.uri(),
                warehouse: "test-warehouse".to_owned(),
                catalog_credential: None,
                catalog_token: None,
            };
            IcebergClient::connect(&config)
                .await
                .expect("connect against wiremock")
        }

        #[tokio::test]
        async fn list_namespaces_returns_the_stubbed_namespaces() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(r#"{"namespaces":[["bronze"],["silver"]]}"#),
                )
                .mount(&server)
                .await;
            let client = connect(&server).await;

            let namespaces = list_namespaces(&client).await.expect("list_namespaces");

            assert_eq!(namespaces.len(), 2);
            assert_eq!(namespaces[0].to_url_string(), "bronze");
            assert_eq!(namespaces[1].to_url_string(), "silver");
        }

        #[tokio::test]
        async fn list_table_idents_returns_the_stubbed_idents() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/bronze/tables"))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"identifiers":[{"namespace":["bronze"],"name":"orders"}]}"#,
                ))
                .mount(&server)
                .await;
            let client = connect(&server).await;
            let namespace = NamespaceIdent::from_strs(["bronze"]).expect("namespace");

            let idents = list_table_idents(&client, &namespace)
                .await
                .expect("list_table_idents");

            assert_eq!(idents.len(), 1);
            assert_eq!(idents[0].name(), "orders");
        }

        #[tokio::test]
        async fn load_table_detail_maps_the_hand_written_fixture() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/bronze/tables/orders"))
                .respond_with(ResponseTemplate::new(200).set_body_string(LOAD_TABLE_FIXTURE))
                .mount(&server)
                .await;
            let client = connect(&server).await;
            let ident = TableIdent::new(
                NamespaceIdent::from_strs(["bronze"]).expect("namespace"),
                "orders".to_owned(),
            );

            let detail = load_table_detail(&client, &ident)
                .await
                .expect("load_table_detail");

            assert_eq!(detail.schema.len(), 2);
            assert_eq!(detail.schema[0].name, "id");
            assert!(detail.schema[0].required);
            assert_eq!(detail.partition_fields.len(), 1);
            assert_eq!(detail.partition_fields[0].source_id, 2);
            assert_eq!(detail.partition_fields[0].transform, "day");
            assert_eq!(detail.partition_fields[0].name, "created_at_day");
            assert_eq!(detail.snapshot_count, 2);
            assert_eq!(detail.metadata_log_count, 1);
            assert_eq!(detail.snapshots.len(), 2);
            // `TableMetadata::snapshots()` iterates a map keyed by snapshot
            // id, not commit order, so look snapshots up by id rather than
            // assuming a position.
            let snapshot_one = detail
                .snapshots
                .iter()
                .find(|s| s.id == 1)
                .expect("snapshot 1 present");
            let snapshot_two = detail
                .snapshots
                .iter()
                .find(|s| s.id == 2)
                .expect("snapshot 2 present");
            assert_eq!(snapshot_one.added_records, Some(100));
            assert_eq!(snapshot_two.deleted_records, Some(0));
            assert_eq!(snapshot_two.total_records, Some(150));
            assert_eq!(snapshot_two.total_data_files, Some(2));
            assert_eq!(detail.stats.record_count, Some(150));
            assert_eq!(detail.stats.total_bytes, Some(2048));
        }

        #[tokio::test]
        async fn load_table_detail_orders_snapshots_oldest_first_by_timestamp() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/bronze/tables/orders"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(LOAD_TABLE_FIXTURE_UNORDERED_IDS),
                )
                .mount(&server)
                .await;
            let client = connect(&server).await;
            let ident = TableIdent::new(
                NamespaceIdent::from_strs(["bronze"]).expect("namespace"),
                "orders".to_owned(),
            );

            let detail = load_table_detail(&client, &ident)
                .await
                .expect("load_table_detail");

            // Snapshot 20 has the smaller timestamp but the LARGER id;
            // snapshot 10 is the newer commit with the SMALLER id. Sorting
            // by id alone would put 10 before 20 — wrong. Sorting by
            // `timestamp_ms` puts 20 (older) first, matching the doc's
            // "oldest first" claim.
            assert_eq!(detail.snapshots.len(), 2);
            assert_eq!(detail.snapshots[0].id, 20);
            assert_eq!(detail.snapshots[1].id, 10);
            assert!(detail.snapshots[0].timestamp_ms < detail.snapshots[1].timestamp_ms);
        }

        #[tokio::test]
        async fn load_table_detail_maps_a_404_to_not_found() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/bronze/tables/missing"))
                .respond_with(ResponseTemplate::new(404).set_body_string(
                    r#"{"error":{"message":"not found","type":"NoSuchTableException","code":404}}"#,
                ))
                .mount(&server)
                .await;
            let client = connect(&server).await;
            let ident = TableIdent::new(
                NamespaceIdent::from_strs(["bronze"]).expect("namespace"),
                "missing".to_owned(),
            );

            let err = load_table_detail(&client, &ident)
                .await
                .expect_err("expected NotFound");

            assert!(matches!(err, RestError::NotFound));
        }

        #[tokio::test]
        async fn list_warehouses_reports_unreachable_on_401() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces"))
                .respond_with(ResponseTemplate::new(401).set_body_string(
                    r#"{"error":{"message":"unauthorized","type":"NotAuthorizedException","code":401}}"#,
                ))
                .mount(&server)
                .await;
            let client = connect(&server).await;

            let warehouses = list_warehouses(&client, "test-warehouse").await;

            assert_eq!(warehouses.len(), 1);
            assert!(!warehouses[0].reachable);
        }

        #[tokio::test]
        async fn list_warehouses_reports_unreachable_on_500() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces"))
                .respond_with(ResponseTemplate::new(500).set_body_string(
                    r#"{"error":{"message":"boom","type":"InternalServerError","code":500}}"#,
                ))
                .mount(&server)
                .await;
            let client = connect(&server).await;

            let warehouses = list_warehouses(&client, "test-warehouse").await;

            assert_eq!(warehouses.len(), 1);
            assert!(!warehouses[0].reachable);
        }

        #[tokio::test]
        async fn list_warehouses_reports_reachable_on_200() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces"))
                .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"namespaces":[]}"#))
                .mount(&server)
                .await;
            let client = connect(&server).await;

            let warehouses = list_warehouses(&client, "test-warehouse").await;

            assert_eq!(warehouses.len(), 1);
            assert!(warehouses[0].reachable);
        }
    }
}
