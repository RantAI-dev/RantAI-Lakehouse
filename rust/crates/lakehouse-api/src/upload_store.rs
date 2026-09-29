//! Object storage for uploaded files — phase 1 of the file-upload
//! feature.
//!
//! An uploaded file's bytes go to the SAME S3 warehouse bucket Bronze
//! already uses, under an `uploads/` prefix. Two reasons, both about
//! keeping the layers honest:
//!
//! 1. **The file itself is the Raw layer.** Keeping it means a parse that
//!    turns out wrong (the wrong delimiter, the wrong header row, a
//!    misread encoding — all of which happen on real exports) can be
//!    redone from the original bytes instead of asking the customer to
//!    send the file again.
//! 2. **`Postgres` holds console state, not customer data.** A 25 MB export
//!    stored as a row is 25 MB the database carries forever and never
//!    queries.
//!
//! # Why `object_store` and not a vendor SDK
//!
//! Same rule `lakehouse-iceberg` states for itself: only the plain S3 API
//! surface, never a storage-admin call. That is what lets this stack run
//! against `RustFS`, `SeaweedFS` or real S3 without a per-vendor branch.

use std::ops::Range;

use axum::body::Bytes;
use lakehouse_core::secret::DynSecretResolver;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::{ObjectStoreExt, PutPayload, path::Path as ObjectPath};

use crate::config::Config;

/// Everything that can go wrong reaching the upload bucket.
#[derive(Debug, thiserror::Error)]
pub enum UploadStoreError {
    /// The S3 client could not be built — a missing credential, or an
    /// endpoint/bucket the builder rejects.
    #[error("upload storage is not configured: {0}")]
    NotConfigured(String),
    /// The store rejected or failed the operation.
    #[error("upload storage error: {0}")]
    Store(#[from] object_store::Error),
}

/// A handle on the `uploads/` prefix of the warehouse bucket.
#[derive(Debug)]
pub struct UploadStore {
    store: AmazonS3,
}

/// The one prefix uploads may be written under. Every key this module
/// builds starts here, so a storage sweep can find every uploaded file
/// without consulting Postgres, and nothing can be written next to the
/// Iceberg data by accident.
pub const PREFIX: &str = "uploads";

impl UploadStore {
    /// Build a client from resolved configuration.
    ///
    /// Credentials come from `RUSTFS_ACCESS_KEY_SECRET_REF` /
    /// `RUSTFS_SECRET_KEY_SECRET_REF` (ADR 0002 `secretRef`s, e.g.
    /// `env:CONNECTOR_S3_ACCESS_KEY`), resolved through the same
    /// `SecretResolver` the rest of the process uses — never a literal key
    /// in config, and never logged: [`UploadStore`]'s `Debug` prints only
    /// the type name because `AmazonS3`'s own `Debug` does not expose the
    /// secret either.
    ///
    /// # Errors
    ///
    /// Returns [`UploadStoreError::NotConfigured`] when a `secretRef` is
    /// absent or cannot be resolved, or when `object_store` rejects the
    /// endpoint/bucket combination.
    pub async fn from_config(
        config: &Config,
        resolver: &dyn DynSecretResolver,
    ) -> Result<Self, UploadStoreError> {
        let access_key = resolve(resolver, config.rustfs_access_key_secret_ref.as_deref()).await?;
        let secret_key = resolve(resolver, config.rustfs_secret_key_secret_ref.as_deref()).await?;
        let store = AmazonS3Builder::new()
            .with_endpoint(config.rustfs_s3_endpoint.clone())
            .with_region(config.rustfs_s3_region.clone())
            .with_bucket_name(config.lakehouse_warehouse_bucket.clone())
            .with_access_key_id(access_key)
            .with_secret_access_key(secret_key)
            // RustFS and SeaweedFS both speak plain HTTP inside the compose
            // network; TLS terminates at the ingress in a real deployment.
            .with_allow_http(true)
            // Path style (`host/bucket/key`), not virtual-hosted
            // (`bucket.host/key`): a bucket-as-subdomain needs DNS entries
            // no self-hosted object store in this stack provides.
            .with_virtual_hosted_style_request(false)
            .build()
            .map_err(|err| UploadStoreError::NotConfigured(err.to_string()))?;
        Ok(Self { store })
    }

    /// Store `bytes` at `key` (which must already be inside [`PREFIX`] —
    /// see `routes::uploads::storage_key`).
    ///
    /// # Errors
    ///
    /// Returns [`UploadStoreError::Store`] when the write fails.
    pub async fn put(&self, key: &str, bytes: Bytes) -> Result<(), UploadStoreError> {
        self.store
            .put(&ObjectPath::from(key), PutPayload::from_bytes(bytes))
            .await?;
        Ok(())
    }

    /// Read at most `max_bytes` from the start of `key`.
    ///
    /// A RANGE read, not a full fetch: a preview needs the first few
    /// kilobytes to sniff an encoding and a delimiter, and pulling a
    /// multi-hundred-megabyte export into the API process to show twenty
    /// rows would be the same mistake as buffering an upload.
    ///
    /// # Errors
    ///
    /// Returns [`UploadStoreError::Store`] when the object is missing or
    /// the read fails.
    pub async fn head_bytes(&self, key: &str, max_bytes: usize) -> Result<Bytes, UploadStoreError> {
        let path = ObjectPath::from(key);
        let size = self.store.head(&path).await?.size;
        let end = u64::try_from(max_bytes).unwrap_or(u64::MAX).min(size);
        if end == 0 {
            return Ok(Bytes::new());
        }
        let range = Range { start: 0, end };
        Ok(self.store.get_range(&path, range).await?)
    }

    /// Delete the object at `key`.
    ///
    /// # Errors
    ///
    /// Returns [`UploadStoreError::Store`] when the delete fails. A
    /// missing object is reported by the store as `NotFound`; callers
    /// deleting a registry row may treat that as success.
    pub async fn delete(&self, key: &str) -> Result<(), UploadStoreError> {
        self.store.delete(&ObjectPath::from(key)).await?;
        Ok(())
    }
}

async fn resolve(
    resolver: &dyn DynSecretResolver,
    secret_ref: Option<&str>,
) -> Result<String, UploadStoreError> {
    let Some(secret_ref) = secret_ref else {
        return Err(UploadStoreError::NotConfigured(
            "RUSTFS_ACCESS_KEY_SECRET_REF / RUSTFS_SECRET_KEY_SECRET_REF are unset".to_owned(),
        ));
    };
    resolver
        .resolve_dyn(secret_ref)
        .await
        .map(|value| value.expose_secret().to_owned())
        .map_err(|err| UploadStoreError::NotConfigured(err.to_string()))
}
