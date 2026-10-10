//! Object storage for uploaded files — T4 of
//! `docs/superpowers/plans/2026-10-02-upload-file.md` (ADR 0014,
//! decisions 1 and 2).
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
//! Same rule `lakehouse-iceberg` states for itself ("`object_store`, never
//! a vendor SDK, never a storage-admin API"): only the plain S3 API
//! surface. That is what lets this stack run against `RustFS`, `SeaweedFS`
//! or real S3 without a per-vendor branch.
//!
//! # Credentials: the shared builder
//!
//! [`UploadStore::connect`] builds its client with
//! [`crate::rustfs_client::build_client`], the same builder the `RustFS`
//! health probe uses, so the rule for resolving
//! `RUSTFS_ACCESS_KEY_SECRET_REF` / `RUSTFS_SECRET_KEY_SECRET_REF` lives in
//! one place. It does not use `AppState::connector_secret_resolver`, which
//! refuses those refs by design.
//!
//! # Errors: two fixed sentences
//!
//! Whatever goes wrong, a caller gets one of two messages, both `503`
//! through `ApiError::Unavailable`:
//!
//! - `Upload storage is not configured.` — the credential refs are unset or
//!   do not resolve, or the endpoint settings are rejected.
//! - `Upload storage is unavailable (<reason>).` — a request failed, and
//!   `<reason>` is the short word
//!   [`crate::connector_probe::classify_object_store_error`] picks
//!   (`connection refused`, `timed out`, `permission denied`, ...).
//!
//! The underlying error is logged (`tracing`, at `warn`) and never
//! returned: `object_store`'s `Display` carries the upstream response body
//! verbatim (`docs/CODE-STANDARD.md` section 1.4). One function,
//! `user_error`, does the mapping, so no call site can return a raw error by
//! accident.

use std::ops::Range;

use axum::body::Bytes;
use lakehouse_core::ApiError;
use object_store::aws::AmazonS3;
use object_store::{ObjectStoreExt, PutPayload, path::Path as ObjectPath};

use crate::config::Config;
use crate::connector_probe::classify_object_store_error;
use crate::rustfs_client::{self, RustfsClientError};

/// The one prefix uploads may be written under. Every key the routes build
/// (`routes::uploads::storage_key`) starts here, so a storage sweep can find
/// every uploaded file without consulting Postgres, and nothing can be
/// written next to the Iceberg data by accident.
pub const PREFIX: &str = "uploads";

/// What a caller of [`UploadStore`] can be told. Exactly two messages, both
/// fixed text: see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UploadStoreError {
    /// The store cannot be used because it is not set up: a credential ref
    /// unset or unresolvable, or endpoint settings `object_store` rejects.
    #[error("Upload storage is not configured.")]
    NotConfigured,
    /// A request to the store failed. The word is the classified reason,
    /// never the upstream text.
    #[error("Upload storage is unavailable ({0}).")]
    Unavailable(&'static str),
}

impl From<UploadStoreError> for ApiError {
    /// Both messages are `503`: the request may well succeed once storage is
    /// configured or back.
    fn from(err: UploadStoreError) -> Self {
        Self::Unavailable(err.to_string())
    }
}

/// What failed, before it is reduced to one of the two messages.
enum Failure<'a> {
    /// The client could not be built.
    Client(&'a RustfsClientError),
    /// A request to the store failed.
    Store(&'a object_store::Error),
}

/// The ONE place an error becomes a message a user can read. Logs the
/// detail, returns the fixed sentence.
fn user_error(operation: &'static str, key: &str, failure: &Failure<'_>) -> UploadStoreError {
    match failure {
        Failure::Client(err) => {
            tracing::warn!(operation, error = %err, "upload storage is not configured");
            UploadStoreError::NotConfigured
        }
        Failure::Store(err) => {
            let class = classify_object_store_error(err);
            tracing::warn!(operation, key, class, error = %err, "upload storage request failed");
            UploadStoreError::Unavailable(class)
        }
    }
}

/// A handle on the `uploads/` prefix of the warehouse bucket.
///
/// Cheap to clone (the client inside is reference-counted) and opens no
/// connection until a request is made.
#[derive(Clone)]
pub struct UploadStore {
    store: AmazonS3,
}

/// `Debug` prints the type name only, by hand: a derived impl would print
/// the whole client, and which of its fields a credential can reach is the
/// dependency's business, not a guarantee this module can make.
impl std::fmt::Debug for UploadStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UploadStore").finish_non_exhaustive()
    }
}

impl UploadStore {
    /// Build the store from the process configuration. Resolves the two
    /// credential refs but opens no connection, so it is cheap enough to
    /// call per request and cannot hang on a dead endpoint.
    ///
    /// # Errors
    ///
    /// [`UploadStoreError::NotConfigured`] when
    /// `RUSTFS_ACCESS_KEY_SECRET_REF` or `RUSTFS_SECRET_KEY_SECRET_REF` is
    /// unset, when a ref does not resolve, or when `object_store` rejects
    /// the endpoint or bucket settings. Nothing is dialled in any of those
    /// cases.
    pub async fn connect(config: &Config) -> Result<Self, UploadStoreError> {
        let store = rustfs_client::build_client(config)
            .await
            .map_err(|err| user_error("connect", "", &Failure::Client(&err)))?;
        Ok(Self { store })
    }

    /// [`Self::connect`] with the source of the credential values injected,
    /// for tests: the allowlist of the two configured refs still applies
    /// (see [`rustfs_client::build_client_with`]).
    ///
    /// # Errors
    ///
    /// As [`Self::connect`].
    #[cfg(test)]
    async fn connect_with<R: lakehouse_core::secret::SecretResolver>(
        config: &Config,
        source: R,
    ) -> Result<Self, UploadStoreError> {
        let store = rustfs_client::build_client_with(config, source)
            .await
            .map_err(|err| user_error("connect", "", &Failure::Client(&err)))?;
        Ok(Self { store })
    }

    /// Store `bytes` at `key` (which must already be inside [`PREFIX`] —
    /// see `routes::uploads::storage_key`).
    ///
    /// # Errors
    ///
    /// [`UploadStoreError::Unavailable`] when the write fails.
    pub async fn put(&self, key: &str, bytes: Bytes) -> Result<(), UploadStoreError> {
        self.store
            .put(&ObjectPath::from(key), PutPayload::from_bytes(bytes))
            .await
            .map_err(|err| user_error("put", key, &Failure::Store(&err)))?;
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
    /// [`UploadStoreError::Unavailable`] when the object is missing (the
    /// reason is `not found`) or the read fails.
    pub async fn head_bytes(&self, key: &str, max_bytes: usize) -> Result<Bytes, UploadStoreError> {
        let path = ObjectPath::from(key);
        let size = self
            .store
            .head(&path)
            .await
            .map_err(|err| user_error("head", key, &Failure::Store(&err)))?
            .size;
        let end = u64::try_from(max_bytes).unwrap_or(u64::MAX).min(size);
        if end == 0 {
            return Ok(Bytes::new());
        }
        let range = Range { start: 0, end };
        self.store
            .get_range(&path, range)
            .await
            .map_err(|err| user_error("get_range", key, &Failure::Store(&err)))
    }

    /// Read the whole object at `key`. Only for what must be read whole: an
    /// Excel workbook cannot be previewed or converted from a prefix. A text
    /// upload is never read whole by the API ([`Self::head_bytes`]).
    ///
    /// # Errors
    ///
    /// [`UploadStoreError::Unavailable`] when the object is missing (the
    /// reason is `not found`) or the read fails.
    pub async fn get_all(&self, key: &str) -> Result<Bytes, UploadStoreError> {
        let result = self
            .store
            .get(&ObjectPath::from(key))
            .await
            .map_err(|err| user_error("get", key, &Failure::Store(&err)))?;
        result
            .bytes()
            .await
            .map_err(|err| user_error("get", key, &Failure::Store(&err)))
    }

    /// Delete the object at `key`. An object that is already gone is
    /// success: the caller is deleting a registry row, and "the bytes are
    /// not there" is the state it wants.
    ///
    /// `object_store` 0.14 sends S3's multi-object delete (`POST
    /// /<bucket>?delete`) for this, not `DELETE /<key>`. `RustFS`
    /// 1.0.0-rc.4 accepts it and reports a key it never held as deleted
    /// (checked against a throwaway container on 2026-10-02, T4 handoff in
    /// the upload plan). A store without multi-object delete can be switched
    /// to the plain call with `AmazonS3Builder::with_disable_bulk_delete`,
    /// which belongs in [`crate::rustfs_client`].
    ///
    /// # Errors
    ///
    /// [`UploadStoreError::Unavailable`] when the delete fails for any other
    /// reason.
    pub async fn delete(&self, key: &str) -> Result<(), UploadStoreError> {
        match self.store.delete(&ObjectPath::from(key)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
            Err(err) => Err(user_error("delete", key, &Failure::Store(&err))),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_core::secret::{EnvSecretResolver, SecretError};
    use wiremock::matchers::{body_bytes, header, header_regex, method, path, query_param};
    use wiremock::{Mock, MockBuilder, MockServer, ResponseTemplate};

    use super::*;

    const KEY: &str = "uploads/tenant-1/up-1.csv";
    const OBJECT_PATH: &str = "/lakehouse-warehouse/uploads/tenant-1/up-1.csv";
    const ACCESS_ID: &str = "AKIDTESTUPLOAD";
    const SECRET_VALUE: &str = "secret-value-for-the-test";
    const UPSTREAM_DETAIL: &str = "secret upstream detail";
    const BODY: &[u8] = b"id,name\n1,a\n2,b\n";

    fn config_for(endpoint: &str) -> Config {
        let map: HashMap<String, String> = [
            ("RUSTFS_S3_ENDPOINT", endpoint),
            ("RUSTFS_ACCESS_KEY_SECRET_REF", "env:TEST_UPLOAD_ACCESS"),
            ("RUSTFS_SECRET_KEY_SECRET_REF", "env:TEST_UPLOAD_SECRET"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        Config::from_map(&map).unwrap()
    }

    fn values() -> EnvSecretResolver {
        EnvSecretResolver::with_map(HashMap::from([
            ("TEST_UPLOAD_ACCESS".to_owned(), ACCESS_ID.to_owned()),
            ("TEST_UPLOAD_SECRET".to_owned(), SECRET_VALUE.to_owned()),
        ]))
    }

    async fn store_for(server: &MockServer) -> UploadStore {
        UploadStore::connect_with(&config_for(&server.uri()), values())
            .await
            .unwrap()
    }

    /// The headers every S3 object response must carry for `object_store`
    /// to read it as metadata.
    fn object_headers(template: ResponseTemplate) -> ResponseTemplate {
        template
            .insert_header("ETag", "\"abc\"")
            .insert_header("Last-Modified", "Wed, 21 Oct 2015 07:28:00 GMT")
    }

    /// `object_store` 0.14 implements `delete` as S3's multi-object delete,
    /// `POST /<bucket>?delete`, not `DELETE /<key>`.
    fn bulk_delete() -> MockBuilder {
        Mock::given(method("POST"))
            .and(path("/lakehouse-warehouse"))
            .and(query_param("delete", ""))
    }

    fn deleted(key: &str) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_string(format!(
            r#"<DeleteResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Deleted><Key>{key}</Key></Deleted></DeleteResult>"#
        ))
    }

    // ── not configured: nothing is dialled ────────────────────────────

    /// T4's acceptance: unset refs mean not configured, and nothing is
    /// dialled. The endpoint is a live mock server, so "nothing" is
    /// something this test could see, not an unreachable address.
    #[tokio::test]
    async fn unset_refs_mean_not_configured_and_nothing_is_dialled() {
        let server = MockServer::start().await;
        for refs in [
            vec![],
            vec![("RUSTFS_ACCESS_KEY_SECRET_REF", "env:TEST_UPLOAD_ACCESS")],
            vec![("RUSTFS_SECRET_KEY_SECRET_REF", "env:TEST_UPLOAD_SECRET")],
        ] {
            let mut map: HashMap<String, String> =
                HashMap::from([("RUSTFS_S3_ENDPOINT".to_owned(), server.uri())]);
            map.extend(refs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
            let config = Config::from_map(&map).unwrap();

            let err = UploadStore::connect(&config).await.unwrap_err();
            assert_eq!(err, UploadStoreError::NotConfigured, "{refs:?}");
            let err = UploadStore::connect_with(&config, values())
                .await
                .unwrap_err();
            assert_eq!(err, UploadStoreError::NotConfigured, "{refs:?}");
        }
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "no request may reach the endpoint of a store that is not configured"
        );
    }

    #[tokio::test]
    async fn refs_that_do_not_resolve_mean_not_configured_and_nothing_is_dialled() {
        let server = MockServer::start().await;
        let config = config_for(&server.uri());

        let nothing_set = EnvSecretResolver::with_map(HashMap::new());
        let err = UploadStore::connect_with(&config, nothing_set)
            .await
            .unwrap_err();
        assert_eq!(err, UploadStoreError::NotConfigured);

        // `connect` reads the real process environment: the two test refs
        // name variables nothing sets, so it fails the same way.
        let err = UploadStore::connect(&config).await.unwrap_err();
        assert_eq!(err, UploadStoreError::NotConfigured);

        assert!(server.received_requests().await.unwrap().is_empty());
    }

    // ── the two fixed messages ────────────────────────────────────────

    #[test]
    fn the_two_messages_are_fixed_sentences_and_both_are_503() {
        assert_eq!(
            UploadStoreError::NotConfigured.to_string(),
            "Upload storage is not configured."
        );
        assert_eq!(
            UploadStoreError::Unavailable("timed out").to_string(),
            "Upload storage is unavailable (timed out)."
        );
        for err in [
            UploadStoreError::NotConfigured,
            UploadStoreError::Unavailable("permission denied"),
        ] {
            let api = ApiError::from(err);
            assert_eq!(api.status(), 503, "{err}");
            assert_eq!(api.to_string(), err.to_string());
        }
    }

    /// Every failure the store can see maps through `user_error` to one of
    /// the two messages; the classified word is the one
    /// `classify_object_store_error` picks, and no source text survives.
    #[test]
    fn every_failure_maps_to_one_of_two_messages_without_the_upstream_text() {
        let generic = |text: &str| object_store::Error::Generic {
            store: "S3",
            source: text.to_owned().into(),
        };
        let store_cases: Vec<(object_store::Error, &str)> = vec![
            (
                object_store::Error::NotFound {
                    path: KEY.to_owned(),
                    source: UPSTREAM_DETAIL.into(),
                },
                "not found",
            ),
            (
                object_store::Error::PermissionDenied {
                    path: KEY.to_owned(),
                    source: UPSTREAM_DETAIL.into(),
                },
                "permission denied",
            ),
            (
                object_store::Error::Unauthenticated {
                    path: KEY.to_owned(),
                    source: UPSTREAM_DETAIL.into(),
                },
                "authentication failed",
            ),
            (
                object_store::Error::NotSupported {
                    source: UPSTREAM_DETAIL.into(),
                },
                "not supported",
            ),
            // Anything `object_store` has no variant for, with the upstream
            // response body in its text, falls to the generic word.
            (generic(UPSTREAM_DETAIL), "connection failed"),
        ];
        for (err, word) in &store_cases {
            assert!(
                err.to_string().contains(UPSTREAM_DETAIL),
                "the fixture error must carry the text this test proves is dropped"
            );
            let mapped = user_error("put", KEY, &Failure::Store(err));
            assert_eq!(mapped, UploadStoreError::Unavailable(word));
            assert_eq!(
                mapped.to_string(),
                format!("Upload storage is unavailable ({word}).")
            );
            assert!(!mapped.to_string().contains(UPSTREAM_DETAIL));
            assert!(!format!("{mapped:?}").contains(UPSTREAM_DETAIL));
        }

        let client_cases = [
            RustfsClientError::NotConfigured,
            RustfsClientError::CredentialUnavailable(SecretError::NotFound {
                secret_ref: "env:TEST_UPLOAD_ACCESS".to_owned(),
                resolver: "env",
            }),
            RustfsClientError::Misconfigured(generic(UPSTREAM_DETAIL)),
        ];
        for err in &client_cases {
            let mapped = user_error("connect", "", &Failure::Client(err));
            assert_eq!(mapped, UploadStoreError::NotConfigured, "{err:?}");
            assert_eq!(mapped.to_string(), "Upload storage is not configured.");
        }
    }

    /// `Debug` never prints the client, so no field of it can print a
    /// credential.
    #[tokio::test]
    async fn debug_prints_the_type_name_and_no_credential() {
        let server = MockServer::start().await;
        let store = store_for(&server).await;
        let printed = format!("{store:?}");
        assert_eq!(printed, "UploadStore { .. }");
        assert!(!printed.contains(ACCESS_ID));
        assert!(!printed.contains(SECRET_VALUE));
    }

    // ── against a stand-in for the bucket ─────────────────────────────

    /// `put`, `head_bytes` and `delete` reach the warehouse bucket,
    /// path-style, under the key they were given, signed with the resolved
    /// access key, and never send the secret key itself. `head_bytes` asks
    /// for a RANGE, not the whole object.
    #[tokio::test]
    async fn put_head_bytes_and_delete_reach_the_bucket_under_the_key() {
        let server = MockServer::start().await;
        let body = BODY.to_vec();
        let size = body.len();
        Mock::given(method("PUT"))
            .and(path(OBJECT_PATH))
            .and(body_bytes(body.clone()))
            .and(header_regex(
                "Authorization",
                &format!("Credential={ACCESS_ID}/"),
            ))
            .respond_with(object_headers(ResponseTemplate::new(200)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("HEAD"))
            .and(path(OBJECT_PATH))
            .respond_with(object_headers(
                ResponseTemplate::new(200).insert_header("Content-Length", size.to_string()),
            ))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(OBJECT_PATH))
            .and(header("Range", "bytes=0-7"))
            .respond_with(object_headers(
                ResponseTemplate::new(206)
                    .insert_header("Content-Range", format!("bytes 0-7/{size}"))
                    .set_body_bytes(body[..8].to_vec()),
            ))
            .expect(1)
            .mount(&server)
            .await;
        bulk_delete()
            .respond_with(deleted(KEY))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        store.put(KEY, Bytes::from(body.clone())).await.unwrap();
        let head = store.head_bytes(KEY, 8).await.unwrap();
        assert_eq!(&head[..], &body[..8]);
        store.delete(KEY).await.unwrap();

        // Whatever was sent, the secret key was not.
        for request in server.received_requests().await.unwrap() {
            let sent = format!("{:?}{:?}", request.headers, request.body);
            assert!(!sent.contains(SECRET_VALUE), "the secret key was sent");
        }
        // The `expect(1)`s above are verified when `server` drops.
    }

    /// A range past the end of a small object is clamped to its size, and an
    /// empty object is not read at all.
    #[tokio::test]
    async fn head_bytes_never_asks_for_more_than_the_object_holds() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path(OBJECT_PATH))
            .respond_with(object_headers(
                ResponseTemplate::new(200).insert_header("Content-Length", "5"),
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(OBJECT_PATH))
            .and(header("Range", "bytes=0-4"))
            .respond_with(object_headers(
                ResponseTemplate::new(206)
                    .insert_header("Content-Range", "bytes 0-4/5")
                    .set_body_bytes(b"a,b\n1".to_vec()),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        let head = store.head_bytes(KEY, 1_000_000).await.unwrap();
        assert_eq!(&head[..], b"a,b\n1");
    }

    #[tokio::test]
    async fn head_bytes_of_an_empty_object_reads_no_range() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path(OBJECT_PATH))
            .respond_with(object_headers(
                ResponseTemplate::new(200).insert_header("Content-Length", "0"),
            ))
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        let head = store.head_bytes(KEY, 1024).await.unwrap();
        assert!(head.is_empty());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "only the HEAD: {requests:?}");
    }

    /// T4: a missing object on delete counts as success. S3 itself, and
    /// `RustFS` (checked against a throwaway 1.0.0-rc.4 container, see the
    /// T4 handoff), report a key that was never there as deleted.
    #[tokio::test]
    async fn deleting_a_key_the_store_never_held_is_success_when_it_reports_deleted() {
        let server = MockServer::start().await;
        bulk_delete()
            .respond_with(deleted("uploads/tenant-1/never-existed.csv"))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        store
            .delete("uploads/tenant-1/never-existed.csv")
            .await
            .unwrap();
    }

    /// ... and so is a store that answers "not found" for it.
    #[tokio::test]
    async fn deleting_an_object_that_is_already_gone_is_success_when_the_store_says_not_found() {
        let server = MockServer::start().await;
        bulk_delete()
            .respond_with(ResponseTemplate::new(404).set_body_string(UPSTREAM_DETAIL))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        store.delete(KEY).await.unwrap();
    }

    /// The store answers 403 with a body naming its internals: the caller
    /// gets the classified word and nothing of the body.
    #[tokio::test]
    async fn a_refused_request_surfaces_the_classified_word_and_never_the_response_body() {
        let server = MockServer::start().await;
        for http_method in ["PUT", "HEAD"] {
            Mock::given(method(http_method))
                .and(path(OBJECT_PATH))
                .respond_with(ResponseTemplate::new(403).set_body_string(UPSTREAM_DETAIL))
                .mount(&server)
                .await;
        }
        bulk_delete()
            .respond_with(ResponseTemplate::new(403).set_body_string(UPSTREAM_DETAIL))
            .mount(&server)
            .await;
        let store = store_for(&server).await;

        let errors = [
            store
                .put(KEY, Bytes::from_static(b"a,b\n"))
                .await
                .unwrap_err(),
            store.head_bytes(KEY, 10).await.unwrap_err(),
            store.delete(KEY).await.unwrap_err(),
        ];
        for err in errors {
            assert_eq!(err, UploadStoreError::Unavailable("permission denied"));
            let api = ApiError::from(err);
            assert_eq!(api.status(), 503);
            assert_eq!(
                api.to_string(),
                "Upload storage is unavailable (permission denied)."
            );
            assert!(!api.to_string().contains(UPSTREAM_DETAIL));
        }
    }

    #[tokio::test]
    async fn reading_an_object_that_is_not_there_is_unavailable_with_the_reason_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("HEAD"))
            .and(path(OBJECT_PATH))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let store = store_for(&server).await;
        let err = store.head_bytes(KEY, 10).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "Upload storage is unavailable (not found)."
        );
    }
}
