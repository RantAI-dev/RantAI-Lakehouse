//! Bronze-layer Iceberg client: object storage, catalog, table create + append.
//!
//! # What this crate owns, and why it exists as its own crate
//!
//! P1 of the Lakehouse Foundation plan needs three things nothing else in
//! the workspace provides: an S3 client, an Iceberg REST catalog client
//! (Lakekeeper), and the logic to create a Bronze table and append rows to
//! it. All three are pinned to pre-1.0 dependencies (`iceberg`,
//! `iceberg-catalog-rest`, both `=0.10.1` — see risk R6 in
//! `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`), so they are confined to this
//! one crate: an upgrade of the Iceberg client stack touches this crate
//! only, never a route handler.
//!
//! # Three decisions worth stating up front
//!
//! 1. **`object_store`, never a vendor SDK, never a storage-admin API.**
//!    [`storage`] implements `iceberg`'s own `Storage`/`StorageFactory`
//!    traits directly on top of `object_store::aws::AmazonS3` — there is no
//!    published `object_store`-backed `StorageFactory` for `iceberg-rust`
//!    0.10.x (the upstream backend for S3/GCS/Azure is the
//!    `iceberg-storage-opendal` crate, not `object_store`), so this crate
//!    writes its own. This is intentional, not an oversight: `OpenDAL` and
//!    `object_store` both wrap the same S3 API, so the choice is about
//!    dependency-graph shape, not capability. `object_store` is what
//!    `lakehouse-store`'s neighboring crates already lean toward
//!    stylistically (plain-API HTTP clients, no vendor SDKs — see
//!    `lakehouse-clickhouse`'s module doc), and RustFS/MinIO admin APIs are
//!    off-limits entirely per the task brief: every credential this crate
//!    uses must work against the plain S3 API surface, nothing
//!    RustFS-specific.
//! 2. **Appends only.** `iceberg-rust` 0.10.x has no `UPDATE`/`DELETE`/
//!    compaction support (see [`catalog::BronzeTable::append`]). Do not
//!    read this crate as implying more than it does: no upsert, no
//!    row-level delete, no manifest rewrite. Those are P4/P5 concerns and,
//!    per `docs/plans/CLICKHOUSE-MAINTENANCE-FINDINGS.md`, some of them
//!    (`OPTIMIZE ... MANIFEST`) are not available at all on `ClickHouse` 26.3
//!    either.
//! 3. **Format version 2, always.** Every table this crate creates is
//!    pinned to Iceberg format-version 2 explicitly (not just relying on
//!    `iceberg-rust`'s current V2 default) — see
//!    [`catalog::IcebergClient::create_bronze_table`]. Format-version 3 support in
//!    `iceberg-rust` 0.10.x is incomplete and `ClickHouse`'s `DataLakeCatalog`
//!    engine (the G1 stop condition's other half) is unverified against v3
//!    tables; creating a v3 table here would be a silent compatibility trap
//!    for the `ClickHouse` side of every G1 test.
//!
//! # What this crate does NOT own
//!
//! - Secret resolution. `secretRef` values (catalog credentials, storage
//!   credentials) are resolved by a [`lakehouse_core::secret::SecretResolver`]
//!   passed in by the caller — see ADR 0002. This crate never reads an
//!   environment variable for a credential value itself; it receives
//!   already-resolved strings in [`catalog::IcebergClientConfig`].
//! - Tenant → warehouse mapping. See ADR 0003. The caller supplies the
//!   already-resolved Lakekeeper warehouse identifier; this crate does not
//!   know what a `TENANT_ID` is.
//! - Console/route wiring. `lakehouse-api`'s Gold export route
//!   (`routes::gold`, ADR 0010) is the first real caller of this crate —
//!   see `gold`'s module doc comment for the export shape it builds on.
//!   Bronze ingestion itself (Debezium/dlt) still writes Iceberg directly,
//!   never through this crate or `lakehouse-api`.

pub mod bronze;
pub mod catalog;
pub mod gold;
pub mod storage;

pub use catalog::{BronzeTable, GoldTable, IcebergClient, IcebergClientConfig, IcebergError};
