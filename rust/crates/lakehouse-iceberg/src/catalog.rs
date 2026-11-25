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
//! [`IcebergClientConfig::catalog_credential`] and
//! [`IcebergClientConfig::catalog_token`] (Lakekeeper's authorization —
//! see ADR 0002 and ADR 0011) are handed to `iceberg-catalog-rest` as-is
//! and never stored anywhere else in this crate; `iceberg-catalog-rest`
//! owns any token exchange/refresh internally. R1 (ADR 0011) uses
//! `catalog_token` (a pre-minted static bearer token, sent as-is on every
//! request) — `catalog_credential`'s `OAuth2` client-credential exchange
//! is not exercised by anything in this build.

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::RecordBatch;
use iceberg::io::S3_PATH_STYLE_ACCESS;
use iceberg::spec::{DataFile, DataFileFormat, FormatVersion, NestedField};
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
use iceberg::{Catalog, CatalogBuilder, ErrorKind, NamespaceIdent, TableCreation, TableIdent};
use iceberg_catalog_rest::{
    REST_CATALOG_PROP_URI, REST_CATALOG_PROP_WAREHOUSE, RestCatalogBuilder,
};
use lakehouse_core::secret::SecretValue;
use parquet::file::properties::WriterProperties;
use thiserror::Error;

use crate::bronze;
use crate::gold;
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
    /// Lakekeeper's catalog API. Not used by anything in this build today —
    /// see [`Self::catalog_token`], which R1 (ADR 0011) uses instead.
    /// `iceberg-catalog-rest` supports both; a client-credential exchange
    /// against `POST {catalog_uri}/v1/oauth/tokens` needs Lakekeeper itself
    /// to broker that exchange against a real identity provider, which
    /// this build's principals do not need (they hold a pre-minted static
    /// token already).
    pub catalog_credential: Option<SecretValue>,
    /// A pre-minted static bearer token for Lakekeeper's catalog API,
    /// forwarded to `iceberg-catalog-rest` as its `token` property (used
    /// as-is for every request's `Authorization: Bearer` header, no
    /// exchange, no refresh — see `iceberg_catalog_rest::catalog::Config::
    /// token`). `None` when Lakekeeper's authorization is not enforced.
    /// R1 (ADR 0011): every principal this build's writers authenticate as
    /// holds one of these, minted once by `ops/oidc-mock` at compose
    /// startup.
    pub catalog_token: Option<SecretValue>,
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
        if let Some(token) = &config.catalog_token {
            props.insert("token".to_owned(), token.expose_secret().to_owned());
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

    /// Ensures the flat namespace for `zone` exists, creating it if not.
    /// Idempotent, same shape as [`Self::ensure_bronze_namespace`].
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] for any failure other than the
    /// namespace already existing.
    pub async fn ensure_published_namespace(
        &self,
        zone: gold::PublishZone,
    ) -> Result<NamespaceIdent, IcebergError> {
        let namespace = zone
            .namespace()
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        if self.catalog.namespace_exists(&namespace).await? {
            return Ok(namespace);
        }
        match self
            .catalog
            .create_namespace(&namespace, HashMap::new())
            .await
        {
            Ok(_) => Ok(namespace),
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

    /// Ensures the flat `gold` namespace (ADR 0010) exists. Thin wrapper
    /// over [`Self::ensure_published_namespace`] kept for 100% backward
    /// compatibility (DATA-65).
    ///
    /// # Errors
    ///
    /// See [`Self::ensure_published_namespace`].
    pub async fn ensure_gold_namespace(&self) -> Result<NamespaceIdent, IcebergError> {
        self.ensure_published_namespace(gold::PublishZone::Gold)
            .await
    }

    /// Creates a published export table named `table_name` (sanitized per
    /// `gold::sanitize_mart_name`) in `zone`'s namespace, with
    /// `domain_fields` plus the standard `_exported_at` column, partitioned
    /// by `day(_exported_at)`, **always at Iceberg format-version 2**. DATA-65:
    /// the mechanics are identical for Silver and Gold; only the namespace
    /// differs. Fails if the table already exists — use
    /// [`Self::load_published_table`] for idempotent create-or-load.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Bronze`] if the name/schema/partition spec
    /// cannot be constructed, or [`IcebergError::Catalog`] if the catalog
    /// call fails (including `TableAlreadyExists`).
    pub async fn create_published_table(
        &self,
        zone: gold::PublishZone,
        table_name: &str,
        domain_fields: Vec<NestedField>,
    ) -> Result<GoldTable, IcebergError> {
        let namespace = self.ensure_published_namespace(zone).await?;
        let sanitized = gold::sanitize_mart_name(table_name)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let schema =
            gold::gold_schema(domain_fields).map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let partition_spec = gold::export_day_partition_spec(&schema)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;

        let creation = TableCreation::builder()
            .name(sanitized)
            .schema(schema)
            .partition_spec(partition_spec)
            .format_version(FormatVersion::V2)
            .build();

        let table = self.catalog.create_table(&namespace, creation).await?;
        Ok(GoldTable { table })
    }

    /// Creates a Gold export table — thin wrapper over
    /// [`Self::create_published_table`] with [`gold::PublishZone::Gold`],
    /// kept for backward compatibility (DATA-65).
    ///
    /// # Errors
    ///
    /// See [`Self::create_published_table`].
    pub async fn create_gold_table(
        &self,
        table_name: &str,
        domain_fields: Vec<NestedField>,
    ) -> Result<GoldTable, IcebergError> {
        self.create_published_table(gold::PublishZone::Gold, table_name, domain_fields)
            .await
    }

    /// Loads an existing published table by name from `zone`'s namespace.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] if the table does not exist or the
    /// catalog call fails.
    pub async fn load_published_table(
        &self,
        zone: gold::PublishZone,
        table_name: &str,
    ) -> Result<GoldTable, IcebergError> {
        let namespace = zone
            .namespace()
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let sanitized = gold::sanitize_mart_name(table_name)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let ident = TableIdent::new(namespace, sanitized);
        let table = self.catalog.load_table(&ident).await?;
        Ok(GoldTable { table })
    }

    /// Loads an existing Gold export table — thin wrapper over
    /// [`Self::load_published_table`] with [`gold::PublishZone::Gold`].
    ///
    /// # Errors
    ///
    /// See [`Self::load_published_table`].
    pub async fn load_gold_table(&self, table_name: &str) -> Result<GoldTable, IcebergError> {
        self.load_published_table(gold::PublishZone::Gold, table_name)
            .await
    }

    /// Loads an existing published table by name from `zone`'s namespace,
    /// or `None` when the catalog says the table does not exist (`DATA-49`,
    /// QA `LH-DATA-1-01`).
    ///
    /// The kind is read off the raw [`iceberg::Error`] before
    /// [`IcebergError`]'s conversion keeps only its `Display`; the REST client
    /// gives [`ErrorKind::TableNotFound`] for the catalog's 404. Every other
    /// failure is still an error: a catalog that refuses the caller must not
    /// read as "never published".
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] if the catalog call fails for any
    /// reason other than "table not found", and [`IcebergError::Bronze`] if
    /// the table name or namespace cannot be built.
    pub async fn load_published_table_if_exists(
        &self,
        zone: gold::PublishZone,
        table_name: &str,
    ) -> Result<Option<GoldTable>, IcebergError> {
        let namespace = zone
            .namespace()
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let sanitized = gold::sanitize_mart_name(table_name)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let ident = TableIdent::new(namespace, sanitized);
        match self.catalog.load_table(&ident).await {
            Ok(table) => Ok(Some(GoldTable { table })),
            Err(err) if err.kind() == ErrorKind::TableNotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    /// Loads an existing Gold export table, or `None` when not published.
    /// Thin wrapper over [`Self::load_published_table_if_exists`] with
    /// [`gold::PublishZone::Gold`] (DATA-65).
    ///
    /// # Errors
    ///
    /// See [`Self::load_published_table_if_exists`].
    pub async fn load_gold_table_if_exists(
        &self,
        table_name: &str,
    ) -> Result<Option<GoldTable>, IcebergError> {
        self.load_published_table_if_exists(gold::PublishZone::Gold, table_name)
            .await
    }

    /// Creates the published table if it does not already exist, or loads
    /// it if it does. Every export run after the first hits the `load`
    /// branch — the schema/partition spec passed here only matters for the
    /// table's first creation.
    ///
    /// # Errors
    ///
    /// See [`Self::create_published_table`] and
    /// [`Self::load_published_table`].
    pub async fn create_or_load_published_table(
        &self,
        zone: gold::PublishZone,
        table_name: &str,
        domain_fields: Vec<NestedField>,
    ) -> Result<GoldTable, IcebergError> {
        match self.load_published_table(zone, table_name).await {
            Ok(table) => Ok(table),
            Err(_) => {
                self.create_published_table(zone, table_name, domain_fields)
                    .await
            }
        }
    }

    /// Creates-or-loads a Gold export table — thin wrapper over
    /// [`Self::create_or_load_published_table`] with
    /// [`gold::PublishZone::Gold`] (DATA-65).
    ///
    /// # Errors
    ///
    /// See [`Self::create_or_load_published_table`].
    pub async fn create_or_load_gold_table(
        &self,
        table_name: &str,
        domain_fields: Vec<NestedField>,
    ) -> Result<GoldTable, IcebergError> {
        self.create_or_load_published_table(gold::PublishZone::Gold, table_name, domain_fields)
            .await
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

    /// The table's current Iceberg schema.
    ///
    /// Callers building a `RecordBatch` for [`Self::append`] MUST derive
    /// their Arrow schema from this (via
    /// `iceberg::arrow::schema_to_arrow_schema`), not hand-write one:
    /// `iceberg-rust`'s Parquet writer maps columns to the Iceberg schema
    /// using the `PARQUET:field_id` metadata that conversion attaches to
    /// each Arrow field, and a hand-written schema without that metadata
    /// fails the write with `DataInvalid => Field id N not found in struct
    /// array`.
    #[must_use]
    pub fn schema(&self) -> &iceberg::spec::Schema {
        self.table.metadata().current_schema()
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
        // Every Bronze table is partitioned by `day(_ingested_at)` (see
        // `bronze::ingestion_day_partition_spec`), so the writer needs the
        // partition value for THIS file up front — `iceberg-rust` does not
        // derive it from the data automatically. `bronze::partition_key_for`
        // computes it from `batch`'s `_ingested_at` column; see that
        // function's doc comment for the one-partition-per-batch assumption
        // this makes.
        let partition_key = bronze::partition_key_for(metadata, &batch)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let mut writer: data_file_writer::DataFileWriter<_, _, _> = data_file_writer_builder
            .build(Some(partition_key))
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

/// A loaded or newly-created Gold export table (ADR 0010), ready to
/// append to.
///
/// Same append-only shape as [`BronzeTable`] and for the same reason
/// (`iceberg-rust` 0.10.x has no `UPDATE`/`DELETE`) — see `gold`'s module
/// doc comment for what that means for a mart re-exported on a schedule.
#[derive(Debug)]
pub struct GoldTable {
    table: Table,
}

impl GoldTable {
    /// The table's fully-qualified identifier.
    #[must_use]
    pub fn identifier(&self) -> &TableIdent {
        self.table.identifier()
    }

    /// The table's current format version. Always [`FormatVersion::V2`]
    /// for a table this crate created — see [`IcebergClient::create_gold_table`].
    #[must_use]
    pub fn format_version(&self) -> FormatVersion {
        self.table.metadata().format_version()
    }

    /// The table's current Iceberg schema. Callers building a
    /// `RecordBatch` for [`Self::append`] MUST derive their Arrow schema
    /// from this — see [`BronzeTable::schema`]'s doc comment for why.
    #[must_use]
    pub fn schema(&self) -> &iceberg::spec::Schema {
        self.table.metadata().current_schema()
    }

    /// The id of the table's current snapshot — the one just committed by
    /// the most recent [`Self::commit_staged`] (or [`Self::append`]), or
    /// `None` for a table that was created (via
    /// [`IcebergClient::create_gold_table`]) but never committed to.
    /// `gold_export::export_mart` never creates a table without immediately
    /// committing at least one staged batch to it (a zero-row source mart
    /// is a documented no-op there, per its own doc comment), so in
    /// practice this is only ever `None` for a table this crate has not
    /// written to at all.
    #[must_use]
    pub fn current_snapshot_id(&self) -> Option<i64> {
        self.table.metadata().current_snapshot_id()
    }

    /// The wall-clock time (Unix milliseconds) `Iceberg` recorded when the
    /// table's current snapshot was committed to the catalog — distinct
    /// from `_exported_at` (`gold_export::export_mart`'s own per-batch
    /// column, written by this process at read time, before the commit).
    /// `None` under the same condition as [`Self::current_snapshot_id`].
    #[must_use]
    pub fn current_snapshot_timestamp_ms(&self) -> Option<i64> {
        self.table
            .metadata()
            .current_snapshot()
            .map(|snapshot| snapshot.timestamp_ms())
    }

    /// The value of the `rantai.latest-copy-of` marker property, or `None`
    /// when the table is not a latest-copy table (`DATA-6` D7). Delegates to
    /// [`gold::latest_copy_of`] — the single definition of the marker — so
    /// the export's refusal and any other reader cannot disagree about the
    /// property's name or shape.
    #[must_use]
    pub fn latest_copy_of(&self) -> Option<&str> {
        gold::latest_copy_of(self.table.metadata().properties())
    }

    /// Writes `batch` as one or more Parquet data files to object storage
    /// and returns their [`DataFile`] descriptors, WITHOUT committing an
    /// `Iceberg` snapshot. The files are invisible to every catalog reader
    /// until they are handed to [`Self::commit_staged`].
    ///
    /// This is the write half of [`Self::append`], split out so a multi-batch
    /// publish (DATA-63) can stage a whole publish one bounded batch at a
    /// time — keeping only the small `Vec<DataFile>` descriptors across
    /// batches, never the `RecordBatch` data — and then commit every file
    /// in ONE snapshot. Staging a later batch failing therefore cannot
    /// expose an earlier batch's rows: no snapshot exists until the commit.
    ///
    /// The Parquet writer, file naming, and `day(_exported_at)` partition
    /// value are exactly the ones [`BronzeTable::append`] uses, with the
    /// Gold prefix and Gold partition key (same mechanics as the old
    /// `GoldTable::append` write half). Each `DefaultFileNameGenerator` is
    /// freshly seeded with a UUID, so batches never collide on a file name.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Bronze`] if the partition value cannot be
    /// derived from `batch`'s `_exported_at` column, or
    /// [`IcebergError::Write`] if writing/closing a Parquet data file fails.
    pub async fn stage_batch(&self, batch: RecordBatch) -> Result<Vec<DataFile>, IcebergError> {
        let metadata = self.table.metadata();
        let location_generator = DefaultLocationGenerator::new(metadata)
            .map_err(|e| IcebergError::Write(e.to_string()))?;
        let file_name_generator = DefaultFileNameGenerator::new(
            "gold".to_owned(),
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
        // Every Gold table is partitioned by `day(_exported_at)` (see
        // `gold::export_day_partition_spec`), so the writer needs the
        // partition value for THIS batch up front. `gold::partition_key_for`
        // computes it from `batch`'s `_exported_at` column; see that
        // function's doc comment for the one-partition-per-batch assumption
        // this makes.
        let partition_key = gold::partition_key_for(metadata, &batch)
            .map_err(|e| IcebergError::Bronze(e.to_string()))?;
        let mut writer: data_file_writer::DataFileWriter<_, _, _> = data_file_writer_builder
            .build(Some(partition_key))
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

        Ok(data_files)
    }

    /// Commits every staged `data_files` descriptor in ONE fast-append
    /// transaction — one new `Iceberg` snapshot visible to catalog readers
    /// at one instant, containing all the rows of the whole publish.
    ///
    /// An empty `data_files` is a no-op (no empty snapshot is issued): an
    /// `iceberg-rust` fast append with zero files is invalid, and a publish
    /// that staged nothing has nothing to commit. This is the zero-row
    /// publish case (DATA-63).
    ///
    /// A caller whose staging failed partway through a publish must simply
    /// not call this: dropping the staged descriptors commits nothing, so
    /// no partial copy can ever become visible.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Catalog`] if the commit fails (including a
    /// concurrent-modification conflict — `iceberg-rust` retries those
    /// internally before surfacing an error).
    pub async fn commit_staged(
        &mut self,
        catalog: &dyn Catalog,
        data_files: Vec<DataFile>,
    ) -> Result<(), IcebergError> {
        if data_files.is_empty() {
            return Ok(());
        }
        let txn = Transaction::new(&self.table);
        let action = txn.fast_append().add_data_files(data_files);
        let txn = action.apply(txn).map_err(IcebergError::from)?;
        self.table = txn.commit(catalog).await.map_err(IcebergError::from)?;
        Ok(())
    }

    /// Appends `batch` as one new Parquet data file, in one fast-append
    /// snapshot, committed through `catalog`. Same mechanics as
    /// [`BronzeTable::append`], partitioned by `day(_exported_at)` instead
    /// of `day(_ingested_at)`.
    ///
    /// This is the single-batch convenience path: it stages `batch` with
    /// [`Self::stage_batch`] and commits the resulting files with
    /// [`Self::commit_staged`]. A multi-batch publish must NOT call this per
    /// batch — that would revert DATA-63 and expose a partial copy between
    /// batches. Keep this method for callers that have exactly one batch.
    ///
    /// # Errors
    ///
    /// Returns [`IcebergError::Write`] if writing the Parquet data file
    /// fails, or [`IcebergError::Catalog`] if the commit fails.
    pub async fn append(
        &mut self,
        catalog: &dyn Catalog,
        batch: RecordBatch,
    ) -> Result<(), IcebergError> {
        let data_files = self.stage_batch(batch).await?;
        self.commit_staged(catalog, data_files).await
    }

    /// Reads every row currently visible in the table's latest snapshot —
    /// used to prove the Gold export round trip: rows this crate appended
    /// through Lakekeeper must be readable back through `iceberg-rust`.
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    //! `GoldTable::current_snapshot_id`/`current_snapshot_timestamp_ms` are
    //! thin delegations to `iceberg-rust`'s own (already-tested)
    //! `TableMetadata` accessors — the behavior worth proving there is only
    //! "before any snapshot exists, both are `None`; once one does, both
    //! are `Some`". Those tests build a `Table` directly from an in-memory
    //! `TableMetadata` (via `TableMetadataBuilder`) and
    //! `FileIO::new_with_memory()`.
    //!
    //! The DATA-63 atomic-publish tests below need a real `Transaction::
    //! commit` (staging alone must not create a snapshot; a whole publish
    //! must create exactly one). `iceberg` 0.10.1 keeps its `MemoryCatalog`
    //! builder behind a private module, so those tests use a minimal
    //! `Catalog` double that implements exactly the two calls a transaction
    //! commit makes (`load_table` and `update_table`), backed by the same
    //! in-memory `FileIO` the fixture above uses — no live Lakekeeper /
    //! `RustFS` stack and no new dependency.

    use arrow_array::{ArrayRef, StringArray, TimestampMicrosecondArray};
    use iceberg::Runtime;
    use iceberg::arrow::schema_to_arrow_schema;
    use iceberg::io::FileIO;
    use iceberg::spec::{Operation, Snapshot, SortOrder, Summary, TableMetadataBuilder};
    use iceberg::table::Table;

    use super::*;

    /// A catalog double for the transaction-commit tests. A commit only
    /// calls `load_table` (to refresh its base) and `update_table` (to apply
    /// the staged `TableUpdates`); every other catalog method is unused and
    /// fails loudly rather than fabricating a result.
    #[derive(Debug)]
    struct CommitCatalog {
        current: std::sync::Mutex<Table>,
    }

    fn unsupported<T>() -> iceberg::Result<T> {
        Err(iceberg::Error::new(
            iceberg::ErrorKind::FeatureUnsupported,
            "this test catalog only implements load_table and update_table",
        ))
    }

    #[async_trait::async_trait]
    impl Catalog for CommitCatalog {
        async fn list_namespaces(
            &self,
            _parent: Option<&iceberg::NamespaceIdent>,
        ) -> iceberg::Result<Vec<iceberg::NamespaceIdent>> {
            unsupported()
        }

        async fn create_namespace(
            &self,
            _namespace: &iceberg::NamespaceIdent,
            _properties: HashMap<String, String>,
        ) -> iceberg::Result<iceberg::Namespace> {
            unsupported()
        }

        async fn get_namespace(
            &self,
            _namespace: &iceberg::NamespaceIdent,
        ) -> iceberg::Result<iceberg::Namespace> {
            unsupported()
        }

        async fn namespace_exists(
            &self,
            _namespace: &iceberg::NamespaceIdent,
        ) -> iceberg::Result<bool> {
            unsupported()
        }

        async fn update_namespace(
            &self,
            _namespace: &iceberg::NamespaceIdent,
            _properties: HashMap<String, String>,
        ) -> iceberg::Result<()> {
            unsupported()
        }

        async fn drop_namespace(
            &self,
            _namespace: &iceberg::NamespaceIdent,
        ) -> iceberg::Result<()> {
            unsupported()
        }

        async fn list_tables(
            &self,
            _namespace: &iceberg::NamespaceIdent,
        ) -> iceberg::Result<Vec<iceberg::TableIdent>> {
            unsupported()
        }

        async fn create_table(
            &self,
            _namespace: &iceberg::NamespaceIdent,
            _creation: iceberg::TableCreation,
        ) -> iceberg::Result<Table> {
            unsupported()
        }

        async fn load_table(&self, ident: &iceberg::TableIdent) -> iceberg::Result<Table> {
            let current = self.current.lock().expect("catalog lock is not poisoned");
            assert_eq!(
                current.identifier(),
                ident,
                "the commit asks for this table"
            );
            Ok(current.clone())
        }

        async fn drop_table(&self, _table: &iceberg::TableIdent) -> iceberg::Result<()> {
            unsupported()
        }

        async fn purge_table(&self, _table: &iceberg::TableIdent) -> iceberg::Result<()> {
            unsupported()
        }

        async fn table_exists(&self, _table: &iceberg::TableIdent) -> iceberg::Result<bool> {
            unsupported()
        }

        async fn rename_table(
            &self,
            _src: &iceberg::TableIdent,
            _dest: &iceberg::TableIdent,
        ) -> iceberg::Result<()> {
            unsupported()
        }

        async fn register_table(
            &self,
            _table: &iceberg::TableIdent,
            _metadata_location: String,
        ) -> iceberg::Result<Table> {
            unsupported()
        }

        async fn update_table(&self, commit: iceberg::TableCommit) -> iceberg::Result<Table> {
            let mut current = self.current.lock().expect("catalog lock is not poisoned");
            let staged = commit.apply(current.clone())?;
            *current = staged.clone();
            Ok(staged)
        }
    }

    /// The existing in-memory fixture plus a catalog double seeded with the
    /// same table, so a reader loading through `catalog` and the writer's
    /// `GoldTable` start from identical metadata.
    fn gold_table_with_catalog() -> (GoldTable, CommitCatalog) {
        let table = gold_table_fixture();
        let catalog = CommitCatalog {
            current: std::sync::Mutex::new(table.table.clone()),
        };
        (table, catalog)
    }

    /// A `RecordBatch` matching `table`'s Gold schema: one publish stamp for
    /// every row, plus `rows` region values.
    fn batch_of(table: &GoldTable, rows: usize) -> RecordBatch {
        let arrow_schema =
            Arc::new(schema_to_arrow_schema(table.schema()).expect("arrow schema converts"));
        let exported_at: ArrayRef = Arc::new(TimestampMicrosecondArray::from(vec![
            1_795_168_800_123_456_i64;
            rows
        ]));
        let region: ArrayRef = Arc::new(StringArray::from(vec!["west"; rows]));
        RecordBatch::try_new(arrow_schema, vec![exported_at, region]).expect("record batch builds")
    }

    /// Reloads the table from the catalog and returns (snapshot count, rows
    /// visible). Reading through the catalog is the point: a reader outside
    /// this process sees exactly what the commit made visible.
    async fn snapshot_count_and_rows(
        catalog: &CommitCatalog,
        ident: &iceberg::TableIdent,
    ) -> (usize, usize) {
        let table = catalog
            .load_table(ident)
            .await
            .expect("gold table reloads from the catalog");
        let snapshots = table.metadata().snapshots().len();
        let gold = GoldTable { table };
        let rows: usize = gold
            .read_all()
            .await
            .expect("committed rows read back")
            .iter()
            .map(RecordBatch::num_rows)
            .sum();
        (snapshots, rows)
    }

    /// Mirrors `gold_export::export_mart`'s publish loop: stage each batch
    /// into the small descriptor accumulator, then commit ALL of them once
    /// after the loop. A failure on any batch returns early, dropping the
    /// staged descriptors without ever committing a snapshot — so the
    /// failure behavior the real export has is what these tests exercise.
    async fn publish_batches(
        table: &mut GoldTable,
        catalog: &dyn Catalog,
        batches: &[usize],
        fail_at: Option<usize>,
    ) -> Result<(), ()> {
        let mut staged = Vec::new();
        for (index, &rows) in batches.iter().enumerate() {
            if Some(index) == fail_at {
                return Err(());
            }
            staged.extend(
                table
                    .stage_batch(batch_of(table, rows))
                    .await
                    .map_err(|_| ())?,
            );
        }
        table.commit_staged(catalog, staged).await.map_err(|_| ())
    }

    fn gold_table_fixture() -> GoldTable {
        let schema = gold::gold_schema(vec![NestedField::required(
            gold::FIRST_DOMAIN_FIELD_ID,
            "region",
            iceberg::spec::Type::Primitive(iceberg::spec::PrimitiveType::String),
        )])
        .expect("gold schema builds");
        let partition_spec =
            gold::export_day_partition_spec(&schema).expect("partition spec builds");
        let creation = TableCreation::builder()
            .name("sales".to_owned())
            .location("memory:///gold/sales".to_owned())
            .schema(schema)
            .partition_spec(partition_spec)
            .sort_order(SortOrder::unsorted_order())
            .format_version(FormatVersion::V2)
            .build();
        let metadata = TableMetadataBuilder::from_table_creation(creation)
            .expect("metadata builder from creation")
            .build()
            .expect("metadata builds")
            .metadata;
        let identifier = TableIdent::from_strs(["gold", "sales"]).expect("table ident");
        let table = Table::builder()
            .identifier(identifier)
            .metadata(metadata)
            // A commit needs a metadata location to derive the next one
            // from (`Table::metadata_location_result`). The in-memory file
            // store needs no real file at this path; a real catalog would
            // have written the initial metadata there.
            .metadata_location(
                "memory:///gold/sales/metadata/0-00000000-0000-0000-0000-000000000000.metadata.json"
                    .to_owned(),
            )
            .file_io(FileIO::new_with_memory())
            .runtime(Runtime::current())
            .build()
            .expect("table builds");
        GoldTable { table }
    }

    fn with_one_snapshot(mut gold_table: GoldTable) -> GoldTable {
        let schema_id = gold_table.table.metadata().current_schema().schema_id();
        let snapshot = Snapshot::builder()
            .with_snapshot_id(1)
            .with_sequence_number(1)
            .with_timestamp_ms(
                i64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("system clock after unix epoch")
                        .as_millis(),
                )
                .expect("current time fits in i64 milliseconds"),
            )
            .with_manifest_list("memory:///gold/sales/metadata/snap-1.avro")
            .with_summary(Summary {
                operation: Operation::Append,
                additional_properties: HashMap::new(),
            })
            .with_schema_id(schema_id)
            .build();
        let metadata =
            TableMetadataBuilder::new_from_metadata(gold_table.table.metadata().clone(), None)
                .set_branch_snapshot(snapshot, iceberg::spec::MAIN_BRANCH)
                .expect("add snapshot to main branch")
                .build()
                .expect("metadata builds")
                .metadata;
        gold_table.table = Table::builder()
            .identifier(gold_table.table.identifier().clone())
            .metadata(metadata)
            .file_io(gold_table.table.file_io().clone())
            .runtime(Runtime::current())
            .build()
            .expect("table rebuilds with new metadata");
        gold_table
    }

    #[tokio::test]
    async fn current_snapshot_id_and_timestamp_are_none_before_any_append() {
        let table = gold_table_fixture();
        assert_eq!(table.current_snapshot_id(), None);
        assert_eq!(table.current_snapshot_timestamp_ms(), None);
    }

    #[tokio::test]
    async fn current_snapshot_id_and_timestamp_are_some_after_one_append() {
        let table = with_one_snapshot(gold_table_fixture());
        assert_eq!(table.current_snapshot_id(), Some(1));
        assert!(table.current_snapshot_timestamp_ms().is_some());
    }

    // DATA-63: the whole point of the fix. Three batches staged separately
    // (bounded memory: only descriptors survive between batches) are
    // committed in ONE snapshot; an outside catalog reader sees all seven
    // rows at once, never a prefix of the new copy.
    #[tokio::test]
    async fn three_staged_batches_commit_as_one_snapshot_with_every_row_visible() {
        let (mut table, catalog) = gold_table_with_catalog();
        let ident = table.identifier().clone();

        publish_batches(&mut table, &catalog, &[2, 2, 3], None)
            .await
            .expect("the publish commits");

        let (snapshots, rows) = snapshot_count_and_rows(&catalog, &ident).await;
        assert_eq!(
            snapshots, 1,
            "exactly one Iceberg snapshot for the whole publish, not one per batch"
        );
        assert_eq!(rows, 7, "all three batches' rows are in that one snapshot");
        assert_eq!(
            table.current_snapshot_id(),
            snapshot_id_of(&catalog, &ident).await
        );
    }

    // DATA-63: a failure on batch 2 of a 3-batch publish must leave the
    // table's snapshot unchanged. The first batch's file is written to
    // storage but never committed, so no partial copy is visible to a
    // catalog reader — they still see the previous copy only.
    #[tokio::test]
    async fn a_failure_on_a_later_batch_commits_no_partial_snapshot() {
        let (mut table, catalog) = gold_table_with_catalog();
        let ident = table.identifier().clone();

        // Seed a first copy so "unchanged" is a specific, observable id and
        // row count (a zero-snapshot table would make the assertion weak).
        table
            .append(&catalog, batch_of(&table, 3))
            .await
            .expect("first copy committed");
        let before_id = table
            .current_snapshot_id()
            .expect("first copy has a snapshot");
        let (before_snapshots, before_rows) = snapshot_count_and_rows(&catalog, &ident).await;

        let failed = publish_batches(&mut table, &catalog, &[2, 2, 3], Some(1)).await;
        assert!(failed.is_err(), "a failure on batch 2 aborts the publish");

        let (after_snapshots, after_rows) = snapshot_count_and_rows(&catalog, &ident).await;
        assert_eq!(after_snapshots, before_snapshots);
        assert_eq!(after_rows, before_rows);
        assert_eq!(table.current_snapshot_id(), Some(before_id));
        assert_eq!(
            after_rows, 3,
            "no rows from the failed publish leaked into the table"
        );
    }

    // DATA-63: a zero-row publish stages nothing, so its single commit is a
    // no-op — no empty snapshot, no empty table mutation, no error (an
    // iceberg-rust fast append with zero files is invalid).
    #[tokio::test]
    async fn committing_no_staged_files_creates_no_snapshot() {
        let (mut table, catalog) = gold_table_with_catalog();
        let ident = table.identifier().clone();

        table
            .commit_staged(&catalog, Vec::new())
            .await
            .expect("an empty commit is a no-op");
        publish_batches(&mut table, &catalog, &[], None)
            .await
            .expect("a publish with no batches is a no-op");

        let (snapshots, rows) = snapshot_count_and_rows(&catalog, &ident).await;
        assert_eq!(snapshots, 0, "no empty snapshot was issued");
        assert_eq!(rows, 0);
        assert_eq!(table.current_snapshot_id(), None);
    }

    // Existing single-batch callers keep working: `append` delegates to
    // stage + one commit and still yields exactly one snapshot.
    #[tokio::test]
    async fn append_still_commits_a_single_batch_as_one_snapshot() {
        let (mut table, catalog) = gold_table_with_catalog();
        let ident = table.identifier().clone();

        table
            .append(&catalog, batch_of(&table, 4))
            .await
            .expect("append commits");

        let (snapshots, rows) = snapshot_count_and_rows(&catalog, &ident).await;
        assert_eq!(snapshots, 1);
        assert_eq!(rows, 4);
    }

    /// The snapshot id as a reader of `catalog` sees it (not the writer's
    /// in-memory handle).
    async fn snapshot_id_of(catalog: &CommitCatalog, ident: &iceberg::TableIdent) -> Option<i64> {
        catalog
            .load_table(ident)
            .await
            .expect("gold table reloads")
            .metadata()
            .current_snapshot_id()
    }
}
