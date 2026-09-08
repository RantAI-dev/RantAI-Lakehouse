//! `object_store`-backed implementation of `iceberg`'s `Storage`/
//! `StorageFactory` traits, targeting S3-compatible object stores.
//!
//! # Why this file exists at all
//!
//! `iceberg-rust` 0.10.x ships no built-in S3 `Storage` implementation in
//! the core `iceberg` crate — the upstream S3/GCS/Azure backend lives in
//! the separate `iceberg-storage-opendal` crate, built on `OpenDAL`. The task
//! brief for this crate is explicit that object I/O must go through
//! `object_store`, not `OpenDAL` and never a vendor SDK. `iceberg`'s
//! `Storage`/`StorageFactory` traits (`iceberg::io`) are designed exactly
//! for this: a third party is expected to implement them. This module is
//! that implementation.
//!
//! # Credentials are never held by this module directly
//!
//! [`ObjectStoreS3StorageFactory::build`] receives an
//! [`iceberg::io::StorageConfig`] whose `s3.*` properties
//! (`s3.access-key-id`, `s3.secret-access-key`, `s3.session-token`,
//! `s3.endpoint`, ...) are populated by the REST catalog client from TWO
//! sources merged together: base properties from the catalog's own
//! `/v1/config` response, and, per-table, the `config` map on a
//! `LoadTableResult`/`CreateTableResult` response. The second source is
//! where Lakekeeper's **vended credentials** land when the client sends
//! `X-Iceberg-Access-Delegation: vended-credentials` (see
//! [`catalog::VENDED_CREDENTIALS_HEADER_PROP`](crate::catalog::VENDED_CREDENTIALS_HEADER_PROP)) —
//! this module never calls Lakekeeper's REST API itself and never reads an
//! environment variable for a credential; it only ever sees whatever
//! `iceberg-catalog-rest` handed it in `StorageConfig`. This is the whole
//! point of the G1 test: the credentials an append actually authenticates
//! with are short-lived and server-issued, not a static `RUSTFS_ACCESS_KEY`
//! baked into this client.
//!
//! # One `object_store` client per bucket, lazily built and cached
//!
//! Iceberg paths arrive as full `s3://bucket/key` URIs. `object_store`'s
//! `AmazonS3` client is bound to a single bucket at construction time. A
//! Bronze warehouse only ever touches one bucket in practice, but nothing
//! in the `Storage` trait guarantees that, so this module keys a small
//! cache by bucket name and builds a client on first sight of each one,
//! rather than assuming a single fixed bucket.
//!
//! # Buffered writes, not multipart streaming
//!
//! [`ObjectStoreFileWrite`] buffers the full file in memory and issues one
//! `put` on `close()`, rather than using `object_store`'s multipart upload
//! API. Bronze append data files at this phase are batch-sized (see
//! `bronze.rs`), not streaming-scale, so this is a deliberate simplicity
//! trade-off, not an oversight — revisit if Bronze data files grow past
//! comfortable in-memory buffering (P4/P5 territory, once CDC volume is
//! real).

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use futures::{StreamExt, TryStreamExt};
use iceberg::io::{
    FileMetadata, FileRead, FileWrite, InputFile, OutputFile, S3Config, Storage, StorageConfig,
    StorageFactory,
};
use iceberg::{Error, ErrorKind, Result};
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as ObjectPath;
use object_store::{ObjectStore, ObjectStoreExt, PutPayload};
use serde::{Deserialize, Serialize};

/// Splits an `s3://bucket/key` URI into `(bucket, key)`.
///
/// # Errors
///
/// Returns an error if `path` does not start with `s3://` or has no key
/// component (bucket-only paths are never valid Iceberg file locations).
fn split_s3_path(path: &str) -> Result<(String, String)> {
    let rest = path.strip_prefix("s3://").ok_or_else(|| {
        Error::new(
            ErrorKind::DataInvalid,
            format!("expected an s3:// path, got {path:?}"),
        )
    })?;
    let (bucket, key) = rest.split_once('/').ok_or_else(|| {
        Error::new(
            ErrorKind::DataInvalid,
            format!("s3 path {path:?} has no key component"),
        )
    })?;
    if bucket.is_empty() || key.is_empty() {
        return Err(Error::new(
            ErrorKind::DataInvalid,
            format!("s3 path {path:?} has an empty bucket or key"),
        ));
    }
    Ok((bucket.to_owned(), key.to_owned()))
}

fn map_object_store_err(err: object_store::Error, context: &str) -> Error {
    Error::new(
        ErrorKind::Unexpected,
        format!("object_store error while {context}: {err}"),
    )
    .with_source(err)
}

/// Storage implementation backing S3-compatible object stores through
/// `object_store`.
///
/// Cheap to clone (an `Arc`-wrapped per-bucket client cache), matching the
/// pattern `iceberg`'s own `LocalFsStorage` uses for `new_input`/
/// `new_output`, which need to hand out an owned `Arc<dyn Storage>` from a
/// `&self` method.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct ObjectStoreS3Storage {
    s3_config: S3ConfigOwned,
    #[serde(skip)]
    clients: Arc<Mutex<HashMap<String, Arc<AmazonS3>>>>,
}

impl std::fmt::Debug for ObjectStoreS3Storage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omits `s3_config`'s credential fields — see
        // `S3ConfigOwned`'s own `Debug` impl.
        f.debug_struct("ObjectStoreS3Storage")
            .field("s3_config", &self.s3_config)
            .finish_non_exhaustive()
    }
}

/// A `Debug`/`Serialize`/`Deserialize`-able mirror of `iceberg::io::S3Config`.
///
/// `iceberg::io::S3Config` derives `Debug` (leaking `access_key_id`/
/// `secret_access_key`/`session_token` verbatim on `{:?}`), so this module
/// cannot store it directly in a struct that might ever be `Debug`-printed
/// (`typetag::serde` requires `Debug` on every `Storage` impl). This type
/// holds the same fields with a redacting `Debug`, matching the pattern
/// `lakehouse_api::config::Config` uses for its own secret fields.
#[derive(Clone, Default, Serialize, Deserialize)]
struct S3ConfigOwned {
    endpoint: Option<String>,
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    region: Option<String>,
    enable_virtual_host_style: bool,
    allow_anonymous: bool,
}

const REDACTED: &str = "<redacted>";

impl std::fmt::Debug for S3ConfigOwned {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3ConfigOwned")
            .field("endpoint", &self.endpoint)
            .field(
                "access_key_id",
                &self.access_key_id.as_ref().map(|_| REDACTED),
            )
            .field(
                "secret_access_key",
                &self.secret_access_key.as_ref().map(|_| REDACTED),
            )
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| REDACTED),
            )
            .field("region", &self.region)
            .field("enable_virtual_host_style", &self.enable_virtual_host_style)
            .field("allow_anonymous", &self.allow_anonymous)
            .finish()
    }
}

impl From<&S3Config> for S3ConfigOwned {
    fn from(cfg: &S3Config) -> Self {
        Self {
            endpoint: cfg.endpoint.clone(),
            access_key_id: cfg.access_key_id.clone(),
            secret_access_key: cfg.secret_access_key.clone(),
            session_token: cfg.session_token.clone(),
            region: cfg.region.clone(),
            enable_virtual_host_style: cfg.enable_virtual_host_style,
            allow_anonymous: cfg.allow_anonymous,
        }
    }
}

impl ObjectStoreS3Storage {
    fn client_for_bucket(&self, bucket: &str) -> Result<Arc<AmazonS3>> {
        #[allow(clippy::unwrap_used)] // poisoned mutex means an earlier panic; nothing recoverable.
        let mut clients = self.clients.lock().unwrap();
        if let Some(client) = clients.get(bucket) {
            return Ok(client.clone());
        }

        let mut builder = AmazonS3Builder::new()
            .with_bucket_name(bucket)
            // Path-style, not virtual-hosted-style: this crate only ever
            // targets self-hosted S3-compatible stores (RustFS,
            // SeaweedFS in P2) reached at a plain host:port, which do not
            // have per-bucket DNS entries for virtual-hosted addressing.
            .with_virtual_hosted_style_request(self.s3_config.enable_virtual_host_style);

        if let Some(endpoint) = &self.s3_config.endpoint {
            builder = builder.with_endpoint(endpoint.clone());
        }
        if let Some(region) = &self.s3_config.region {
            builder = builder.with_region(region.clone());
        }
        if self.s3_config.allow_anonymous {
            builder = builder.with_skip_signature(true);
        } else {
            if let Some(key) = &self.s3_config.access_key_id {
                builder = builder.with_access_key_id(key.clone());
            }
            if let Some(secret) = &self.s3_config.secret_access_key {
                builder = builder.with_secret_access_key(secret.clone());
            }
            if let Some(token) = &self.s3_config.session_token {
                builder = builder.with_token(token.clone());
            }
        }
        // `with_endpoint` implies a non-AWS host; `object_store` requires
        // this to be told explicitly it is not talking to real AWS S3, or
        // it refuses self-signed/http endpoints outright.
        if self.s3_config.endpoint.is_some() {
            builder = builder.with_allow_http(true);
        }

        let built = builder.build().map_err(|err| {
            Error::new(
                ErrorKind::Unexpected,
                format!("failed to build object_store S3 client for bucket {bucket:?}: {err}"),
            )
        })?;
        let client = Arc::new(built);
        clients.insert(bucket.to_owned(), client.clone());
        Ok(client)
    }
}

#[async_trait]
#[typetag::serde]
impl Storage for ObjectStoreS3Storage {
    async fn exists(&self, path: &str) -> Result<bool> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        match client.head(&ObjectPath::from(key)).await {
            Ok(_) => Ok(true),
            Err(object_store::Error::NotFound { .. }) => Ok(false),
            Err(err) => Err(map_object_store_err(err, "checking existence")),
        }
    }

    async fn metadata(&self, path: &str) -> Result<FileMetadata> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        let meta = client
            .head(&ObjectPath::from(key))
            .await
            .map_err(|err| map_object_store_err(err, "reading metadata"))?;
        Ok(FileMetadata { size: meta.size })
    }

    async fn read(&self, path: &str) -> Result<Bytes> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        let result = client
            .get(&ObjectPath::from(key))
            .await
            .map_err(|err| map_object_store_err(err, "reading"))?;
        result
            .bytes()
            .await
            .map_err(|err| map_object_store_err(err, "reading body"))
    }

    async fn reader(&self, path: &str) -> Result<Box<dyn FileRead>> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        Ok(Box::new(ObjectStoreFileRead {
            client,
            path: ObjectPath::from(key),
        }))
    }

    async fn write(&self, path: &str, bs: Bytes) -> Result<()> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        client
            .put(&ObjectPath::from(key), PutPayload::from_bytes(bs))
            .await
            .map_err(|err| map_object_store_err(err, "writing"))?;
        Ok(())
    }

    async fn writer(&self, path: &str) -> Result<Box<dyn FileWrite>> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        Ok(Box::new(ObjectStoreFileWrite {
            client,
            path: ObjectPath::from(key),
            buffer: Vec::new(),
            closed: false,
        }))
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        match client.delete(&ObjectPath::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(err) => Err(map_object_store_err(err, "deleting")),
        }
    }

    async fn delete_prefix(&self, path: &str) -> Result<()> {
        let (bucket, key) = split_s3_path(path)?;
        let client = self.client_for_bucket(&bucket)?;
        let prefix = ObjectPath::from(key);
        let mut listing = client.list(Some(&prefix));
        while let Some(entry) = listing
            .try_next()
            .await
            .map_err(|err| map_object_store_err(err, "listing for prefix delete"))?
        {
            client
                .delete(&entry.location)
                .await
                .map_err(|err| map_object_store_err(err, "deleting listed object"))?;
        }
        Ok(())
    }

    async fn delete_stream(&self, mut paths: BoxStream<'static, String>) -> Result<()> {
        while let Some(path) = paths.next().await {
            self.delete(&path).await?;
        }
        Ok(())
    }

    fn new_input(&self, path: &str) -> Result<InputFile> {
        Ok(InputFile::new(Arc::new(self.clone()), path.to_owned()))
    }

    fn new_output(&self, path: &str) -> Result<OutputFile> {
        Ok(OutputFile::new(Arc::new(self.clone()), path.to_owned()))
    }
}

/// [`FileRead`] over an `object_store` client, for one object.
#[derive(Debug)]
struct ObjectStoreFileRead {
    client: Arc<AmazonS3>,
    path: ObjectPath,
}

#[async_trait]
impl FileRead for ObjectStoreFileRead {
    async fn read(&self, range: Range<u64>) -> Result<Bytes> {
        let result = self
            .client
            .get_range(&self.path, range)
            .await
            .map_err(|err| map_object_store_err(err, "ranged reading"))?;
        Ok(result)
    }
}

/// [`FileWrite`] over an `object_store` client. See the module doc for why
/// this buffers in memory instead of using multipart upload.
#[derive(Debug)]
struct ObjectStoreFileWrite {
    client: Arc<AmazonS3>,
    path: ObjectPath,
    buffer: Vec<u8>,
    closed: bool,
}

#[async_trait]
impl FileWrite for ObjectStoreFileWrite {
    async fn write(&mut self, bs: Bytes) -> Result<()> {
        if self.closed {
            return Err(Error::new(
                ErrorKind::Unexpected,
                "cannot write to a closed file",
            ));
        }
        self.buffer.extend_from_slice(&bs);
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        if self.closed {
            return Err(Error::new(ErrorKind::Unexpected, "file already closed"));
        }
        self.closed = true;
        let payload = PutPayload::from_bytes(Bytes::from(std::mem::take(&mut self.buffer)));
        self.client
            .put(&self.path, payload)
            .await
            .map_err(|err| map_object_store_err(err, "closing (final put)"))?;
        Ok(())
    }
}

/// Factory that builds [`ObjectStoreS3Storage`] instances from an
/// [`iceberg::io::StorageConfig`]'s `s3.*` properties.
///
/// Stateless by design: every field the resulting [`ObjectStoreS3Storage`]
/// needs comes from the `StorageConfig` passed to [`Self::build`], not from
/// this factory. That is what lets `iceberg-catalog-rest`'s merge of
/// catalog-level and per-table `config` properties (see the module doc)
/// flow through untouched.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ObjectStoreS3StorageFactory;

#[typetag::serde]
impl StorageFactory for ObjectStoreS3StorageFactory {
    fn build(&self, config: &StorageConfig) -> Result<Arc<dyn Storage>> {
        let s3_config = S3Config::try_from(config)?;
        Ok(Arc::new(ObjectStoreS3Storage {
            s3_config: S3ConfigOwned::from(&s3_config),
            clients: Arc::new(Mutex::new(HashMap::new())),
        }))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn split_s3_path_extracts_bucket_and_key() {
        let (bucket, key) = split_s3_path("s3://my-bucket/a/b/c.parquet").unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "a/b/c.parquet");
    }

    #[test]
    fn split_s3_path_rejects_non_s3_scheme() {
        assert!(split_s3_path("file:///a/b").is_err());
    }

    #[test]
    fn split_s3_path_rejects_bucket_only() {
        assert!(split_s3_path("s3://my-bucket").is_err());
        assert!(split_s3_path("s3://my-bucket/").is_err());
    }

    #[test]
    fn debug_never_renders_credentials() {
        let storage = ObjectStoreS3Storage {
            s3_config: S3ConfigOwned {
                endpoint: Some("http://rustfs:9000".to_owned()),
                access_key_id: Some("AKIA-super-secret".to_owned()),
                secret_access_key: Some("very-secret-key".to_owned()),
                session_token: Some("session-token-value".to_owned()),
                region: Some("us-east-1".to_owned()),
                enable_virtual_host_style: false,
                allow_anonymous: false,
            },
            clients: Arc::new(Mutex::new(HashMap::new())),
        };
        let rendered = format!("{storage:?}");
        for secret in [
            "AKIA-super-secret",
            "very-secret-key",
            "session-token-value",
        ] {
            assert!(!rendered.contains(secret), "leaked {secret:?}: {rendered}");
        }
        assert!(rendered.contains("http://rustfs:9000"));
    }

    #[test]
    fn factory_builds_from_storage_config_props() {
        let config = StorageConfig::new()
            .with_prop("s3.endpoint", "http://rustfs:9000")
            .with_prop("s3.access-key-id", "ak")
            .with_prop("s3.secret-access-key", "sk")
            .with_prop("s3.region", "us-east-1")
            .with_prop("s3.path-style-access", "true");
        let factory = ObjectStoreS3StorageFactory;
        let storage = factory.build(&config).unwrap();
        assert!(format!("{storage:?}").contains("http://rustfs:9000"));
    }
}
