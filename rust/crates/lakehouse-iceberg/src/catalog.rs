//! Lakekeeper Iceberg REST catalog client: connect, create a Bronze table,
//! append rows.
//!
//! # The one header that makes G1 mean anything
//!
//! [`IcebergClient::connect`] always sets
//! `header.x-iceberg-access-delegation: vended-credentials` as a base
//! property on the REST catalog client (see [`VENDED_CREDENTIALS_HEADER_PROP`]).
//! `iceberg-catalog-rest` 0.10.1 does not send this header on its own —
//! confirmed by reading `RestCatalog`'s request-building code, which has no
//! reference to access delegation at all. Iceberg REST catalog servers
//! (Lakekeeper included) only include short-lived storage credentials in a
//! `loadTable`/`createTable` response's `config` map when the client asks
//! for them this way; without the header, Lakekeeper would only ever
//! return metadata, and this crate's `object_store` client
//! ([`crate::storage`]) would have nothing to authenticate S3 calls with.
//! Setting it unconditionally (not behind a flag) is deliberate: there is
//! no code path in this crate that is correct without vended credentials,
//! so there is no reason to make asking for them optional.
//!
//! # No caching of the resolved catalog credential
//!
//! [`IcebergClientConfig::catalog_credential`] (Lakekeeper's own `OAuth2`
//! client-credential, when authorization is enabled — see ADR 0002 and the
//! P1b report's R1 finding) is handed to `iceberg-catalog-rest` as-is and
//! never stored anywhere else in this crate; `iceberg-catalog-rest` owns
//! the token exchange and refresh internally.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::RecordBatch;
use iceberg::io::S3_PATH_STYLE_ACCESS;
use iceberg::spec::{DataFileFormat, FormatVersion, NestedField};
use iceberg::table::Table;
use iceberg::transaction::{ApplyTransactionAction, Transaction};
use iceberg::writer::IcebergWriter;
use iceberg::writer::base_writer::data_file_writer::DataFileWriterBuilder;
use iceberg::writer::file_writer::ParquetWriterBuilder;
use iceberg::writer::file_writer::location_generator::{
    DefaultFileNameGenerator, DefaultLocationGenerator,
};
use iceberg::writer::file_writer::rolling_writer::RollingFileWriterBuilder;
use iceberg::writer::{IcebergWriterBuilder, base_writer::data_file_writer};
use iceberg::{Catalog, CatalogBuilder, NamespaceIdent, TableCreation, TableIdent};
use iceberg_catalog_rest::{
    REST_CATALOG_PROP_URI, REST_CATALOG_PROP_WAREHOUSE, RestCatalogBuilder,
};
use lakehouse_core::secret::SecretValue;
use parquet::file::properties::WriterProperties;
use thiserror::Error;

use crate::bronze;
use crate::storage::ObjectStoreS3StorageFactory;

/// Iceberg REST catalog property that requests vended storage credentials
/// on every `loadTable`/`createTable` response. See the module doc.
pub const VENDED_CREDENTIALS_HEADER_PROP: &str = "header.x-iceberg-access-delegation";
/// Value the Iceberg REST spec defines for requesting vended credentials
/// (as opposed to `remote-signing`, which this crate deliberately does not
/// request — see ADR 0002's discussion of why credential vending, not
/// remote signing, is the mechanism this crate proves).
pub const VENDED_CREDENTIALS_HEADER_VALUE: &str = "vended-credentials";

/// Errors constructing or using an [`IcebergClient`].
#[derive(Debug, Error)]
pub enum IcebergError {
    /// The REST catalog client could not be built or an operation against
    /// it failed. Wraps `iceberg::Error` (the underlying error, from either
    /// `iceberg` or `iceberg-catalog-rest`, does not implement
    /// `std::error::Error` in a way `thiserror`'s `#[from]` can use
    /// directly across the `async fn`-in-trait boundary this crate's error
    /// sites cross, so it is captured as its `Display` rendering instead of
    /// losing the message).
    #[error("iceberg catalog operation failed: {0}")]
    Catalog(String),
    /// A Bronze table/schema construction step failed (naming, partition
    /// spec, etc. — see `bronze.rs`).
    #[error("bronze table setup failed: {0}")]
    Bronze(String),
    /// Writing or reading Arrow/Parquet data for an append failed.
    #[error("data file write failed: {0}")]
    Write(String),
}

impl From<iceberg::Error> for IcebergError {
    fn from(err: iceberg::Error) -> Self {
        Self::Catalog(err.to_string())
    }
}

/// Everything [`IcebergClient::connect`] needs. Every field here is already
/// resolved — see the crate-level doc comment's "What this crate does NOT
/// own" section: secret resolution (ADR 0002) and tenant → warehouse
/// mapping (ADR 0003) both happen before this struct is built, not inside
/// this crate.
#[derive(Clone)]
pub struct IcebergClientConfig {
    /// Lakekeeper's Iceberg REST catalog base URI, e.g.
    /// `http://lakekeeper:8181/catalog`.
    pub catalog_uri: String,
    /// The Lakekeeper warehouse identifier this client operates against —
    /// already resolved from a tenant id per ADR 0003, never a raw
    /// `TENANT_ID` itself.
    pub warehouse: String,
    /// `OAuth2` client-credential (`"client_id:client_secret"`) for
    /// Lakekeeper's catalog API, when Lakekeeper authorization is enabled.
    /// `None` when Lakekeeper is running with no-auth (open) mode — see the
    /// P1b report for whether that is the case in this deployment.
    pub catalog_credential: Option<SecretValue>,
}

/// A connected Lakekeeper Iceberg REST catalog client, with the
/// `object_store`-backed [`ObjectStoreS3StorageFactory`] wired in.
#[derive(Debug)]
pub struct IcebergClient {
    catalog: iceberg_catalog_rest::RestCatalog,
}

impl IcebergClient {
    /// Connect to Lakekeeper.
    ///
    /// Does not touch the network beyond `iceberg-catalog-rest`'s own
    /// `GET /v1/config` call (which `RestCatalogBuilder::load` performs to
    /// resolve server-advertised defaults/overrides) — no namespace or
    /// table is created here.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] if the REST catalog client cannot
    /// be built (e.g. Lakekeeper is unreachable, or `config.catalog_uri` is
    /// empty).
    pub async fn connect(config: &IcebergClientConfig) -> Result<Self, IcebergError> {
        let mut props = HashMap::from([
            (REST_CATALOG_PROP_URI.to_owned(), config.catalog_uri.clone()),
            (
                REST_CATALOG_PROP_WAREHOUSE.to_owned(),
                config.warehouse.clone(),
            ),
            (
                VENDED_CREDENTIALS_HEADER_PROP.to_owned(),
                VENDED_CREDENTIALS_HEADER_VALUE.to_owned(),
            ),
            // RustFS (and SeaweedFS in P2) are reached at a plain
            // host:port with no per-bucket DNS entry, so path-style
            // addressing is required — this is a base default; a vended
            // per-table `config` value for the same key, if Lakekeeper
            // ever sends one, still wins (see `storage.rs`'s module doc on
            // how the REST client merges catalog-level and per-table
            // config).
            (S3_PATH_STYLE_ACCESS.to_owned(), "true".to_owned()),
        ]);
        if let Some(credential) = &config.catalog_credential {
            props.insert(
                "credential".to_owned(),
                credential.expose_secret().to_owned(),
            );
        }

        let catalog = RestCatalogBuilder::default()
            .with_storage_factory(Arc::new(ObjectStoreS3StorageFactory))
            .load("lakekeeper", props)
            .await
            .map_err(IcebergError::from)?;

        Ok(Self { catalog })
    }

    /// Ensures the flat `bronze` namespace exists, creating it if not.
    ///
    /// Idempotent: an already-existing namespace is not an error.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] for any failure other than the
    /// namespace already existing.
    pub async fn ensure_bronze_namespace(&self) -> Result<NamespaceIdent, IcebergError> {
        let namespace =
            bronze::bronze_namespace().map_err(|e| IcebergError::Bronze(e.to_string()))?;
        if self.catalog.namespace_exists(&namespace).await? {
            return Ok(namespace);
        }
        match self
            .catalog
            .create_namespace(&namespace, HashMap::new())
            .await
        {
            Ok(_) => Ok(namespace),
            // Racing another process that created it first is fine; check
            // once more rather than assuming that's what happened.
            Err(err)
                if self
                    .catalog
                    .namespace_exists(&namespace)
                    .await
                    .unwrap_or(false) =>
            {
                let _ = err;
                Ok(namespace)
            }
            Err(err) => Err(IcebergError::from(err)),
        }
    }

    /// Creates a Bronze table named `table_name` (sanitized per
    /// `bronze::sanitize_table_name`) with `domain_fields` plus the
    /// standard `_ingested_at` column, partitioned by `day(_ingested_at)`,
    /// **always at Iceberg format-version 2** — see the crate-level doc
    /// comment's decision #3. Fails if the table already exists (use
    /// [`Self::load_bronze_table`] for idempotent create-or-load).
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Bronze`] if the name/schema/partition spec
    /// cannot be constructed, or [`IcebergError::Catalog`] if the catalog
    /// call fails (including `TableAlreadyExists`).
    pub async fn create_bronze_table(
        &self,
        table_name: &str,
        domain_fields: Vec<NestedField>,
    ) -> Result<BronzeTable, IcebergError> {
        let namespace = self.ensure_bronze_namespace().await?;
        let sanitized = bronze::sanitize_table_name(table_name)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let schema = bronze::bronze_schema(domain_fields)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let partition_spec = bronze::ingestion_day_partition_spec(&schema)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;

        let creation = TableCreation::builder()
            .name(sanitized)
            .schema(schema)
            .partition_spec(partition_spec)
            .format_version(FormatVersion::V2)
            .build();

        let table = self.catalog.create_table(&namespace, creation).await?;
        Ok(BronzeTable { table })
    }

    /// Loads an existing Bronze table by name.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] if the table does not exist or the
    /// catalog call fails.
    pub async fn load_bronze_table(&self, table_name: &str) -> Result<BronzeTable, IcebergError> {
        let namespace =
            bronze::bronze_namespace().map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let sanitized = bronze::sanitize_table_name(table_name)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let ident = TableIdent::new(namespace, sanitized);
        let table = self.catalog.load_table(&ident).await?;
        Ok(BronzeTable { table })
    }

    /// Borrows the underlying [`Catalog`] — needed because
    /// [`Transaction::commit`] takes `&dyn Catalog`, and [`BronzeTable`]
    /// (deliberately) does not hold a reference back to the client that
    /// loaded it.
    #[must_use]
    pub fn as_catalog(&self) -> &dyn Catalog {
        &self.catalog
    }
}

/// A loaded or newly-created Bronze table, ready to append to.
///
/// Deliberately append-only: `iceberg-rust` 0.10.x has no `UPDATE`/
/// `DELETE`/compaction support at the table level, so this type exposes
/// exactly one write operation (plus a read-back used by the G1 test to
/// prove half (b): rows `ClickHouse` writes through the catalog must be
/// readable back via `iceberg-rust`).
#[derive(Debug)]
pub struct BronzeTable {
    table: Table,
}

impl BronzeTable {
    /// The table's fully-qualified identifier.
    #[must_use]
    pub fn identifier(&self) -> &TableIdent {
        self.table.identifier()
    }

    /// The table's current format version, as recorded in its metadata.
    /// Used by the G1 test to assert format-version 2 directly from table
    /// metadata rather than only from the request this crate sent.
    #[must_use]
    pub fn format_version(&self) -> FormatVersion {
        self.table.metadata().format_version()
    }

    /// Appends `batch` as one new Parquet data file, in one fast-append
    /// snapshot, committed through `catalog`.
    ///
    /// `catalog` is taken as a parameter (rather than this crate storing
    /// one on `BronzeTable`) because `Transaction::commit` itself takes
    /// `&dyn Catalog` — pass `client.as_catalog()`.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Write`] if writing the Parquet data file
    /// fails, or [`IcebergError::Catalog`] if the commit fails (including a
    /// concurrent-modification conflict — `iceberg-rust` retries those
    /// internally per its configured backoff before surfacing an error).
    pub async fn append(
        &mut self,
        catalog: &dyn Catalog,
        batch: RecordBatch,
    ) -> Result<(), IcebergError> {
        let metadata = self.table.metadata();
        let location_generator = DefaultLocationGenerator::new(metadata)
            .map_err(|e| IcebergError::Write(e.to_string()))?;
        let file_name_generator = DefaultFileNameGenerator::new(
            "bronze".to_owned(),
            Some(uuid::Uuid::new_v4().to_string()),
            DataFileFormat::Parquet,
        );

        let parquet_writer_builder = ParquetWriterBuilder::new(
            WriterProperties::builder().build(),
            metadata.current_schema().clone(),
        );
        let rolling_writer_builder = RollingFileWriterBuilder::new_with_default_file_size(
            parquet_writer_builder,
            self.table.file_io().clone(),
            location_generator,
            file_name_generator,
        );
        let data_file_writer_builder = DataFileWriterBuilder::new(rolling_writer_builder);
        let mut writer: data_file_writer::DataFileWriter<_, _, _> = data_file_writer_builder
            .build(None)
            .await
            .map_err(|e| IcebergError::Write(e.to_string()))?;

        writer
            .write(batch)
            .await
            .map_err(|e| IcebergError::Write(e.to_string()))?;
        let data_files = writer
            .close()
            .await
            .map_err(|e| IcebergError::Write(e.to_string()))?;

        let txn = Transaction::new(&self.table);
        let action = txn.fast_append().add_data_files(data_files);
        let txn = action.apply(txn).map_err(IcebergError::from)?;
        self.table = txn.commit(catalog).await.map_err(IcebergError::from)?;

        Ok(())
    }

    /// Reads every row currently visible in the table's latest snapshot.
    ///
    /// Used by the G1 test to prove half (b): rows `ClickHouse` writes
    /// through Lakekeeper (not path-based) must be readable back through
    /// this crate's `iceberg-rust`-backed client, over the same
    /// `object_store` `Storage` implementation this crate uses for writes.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Write`] if the scan cannot be built or the
    /// underlying Parquet/Arrow read fails.
    pub async fn read_all(&self) -> Result<Vec<RecordBatch>, IcebergError> {
        let scan = self
            .table
            .scan()
            .build()
            .map_err(|e| IcebergError::Write(e.to_string()))?;
        let stream = scan
            .to_arrow()
            .await
            .map_err(|e| IcebergError::Write(e.to_string()))?;
        futures::TryStreamExt::try_collect(stream)
            .await
            .map_err(|e| IcebergError::Write(e.to_string()))
    }
}
