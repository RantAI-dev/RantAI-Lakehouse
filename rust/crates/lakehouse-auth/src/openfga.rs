//! A thin client over `Lakekeeper`'s own `management/v1/{warehouse,
//! permissions}` REST endpoints, used by `POST /api/identity/tenants` to
//! provision a tenant's warehouse and grant this stack's existing
//! machine principals onto it.
//!
//! # Why this file is named `openfga.rs` but contains no `OpenFGA` client
//!
//! Verified before writing a line of it: `grep -rn
//! "openfga" rust/crates/*/src/**/*.rs` across this whole workspace
//! returns zero hits, and every grant this build has ever made —
//! `docker-compose.yml`'s `lakekeeper-authz-init` job — goes through
//! `Lakekeeper`'s `management/v1/permissions/warehouse/{id}/assignments`
//! endpoint with an admin-scoped bearer token, never `OpenFGA`'s own gRPC
//! or HTTP API. `LAKEKEEPER__AUTHZ_BACKEND: openfga`
//! (`docker-compose.yml:348,539`) makes `OpenFGA` `Lakekeeper`'s *internal*
//! store, invisible to every caller outside `Lakekeeper` itself. This
//! module is that `Lakekeeper`-facing client; its actual contents match
//! what this codebase's one proven grant mechanism (the
//! `lakekeeper-authz-init` shell script in `docker-compose.yml`) does.
//!
//! # Credential handling
//!
//! The admin bearer token is the caller's responsibility to read and
//! wrap in [`crate::secret::Secret`] before constructing
//! [`LakekeeperAdminClient`] (a later task,
//! `Config::lakekeeper_admin_token_file`, mirrors
//! `Config::lakekeeper_gold_export_token_file`'s existing pattern for
//! that read) — this client never logs it, never places it in a
//! `tracing` field, and never includes it (or any upstream response
//! body) in an error returned to a caller: every failure here is
//! classified into a fixed message by [`OpenfgaError`].
//!
//! # Dialing
//!
//! This crate's only other outbound HTTP dialer, [`crate::oidc`]'s JWKS
//! fetch, dials a plain `reqwest::Client` because its target is
//! operator-supplied config (the OIDC issuer), never attacker-influenced
//! input — the allowlisted resolver/`resolve_checked` path is for routes
//! that dial a URL a tenant or connector definition supplied (see
//! `lakehouse-api::connector_probe`). `base_url` here is the same shape
//! as the OIDC issuer: it comes from this stack's own deployment config,
//! not from a request body, so this client follows `oidc.rs`'s
//! precedent rather than introducing a second dialing pattern into this
//! crate.

use serde::Deserialize;
use serde_json::json;

use crate::secret::Secret;

/// The five machine principals every provisioned warehouse needs granted
/// onto it so the stack's existing services can actually read/write a
/// new tenant's data — see the module doc comment for why this is a
/// fixed list, not the calling user's own identity: these are the
/// service accounts every warehouse needs regardless of who provisioned
/// it. Matches `docker-compose.yml`'s `lakekeeper-authz-init`
/// `grant()` calls exactly (verb sets included).
const MACHINE_GRANTS: &[(&str, &[&str])] = &[
    ("rust-iceberg", &["create", "modify", "select"]),
    ("debezium", &["create", "modify", "select"]),
    ("dlt", &["create", "modify", "select"]),
    ("clickhouse-reader", &["select", "modify"]),
    ("trino", &["select", "modify"]),
];

/// A fixed, upstream-body-free classification of everything that can go
/// wrong talking to `Lakekeeper`'s management API. AGENTS.md rule: upstream
/// error text never reaches a response — no variant here carries a
/// `String` built from a `reqwest`/`serde_json` error or a response body.
#[derive(Debug, thiserror::Error)]
pub enum OpenfgaError {
    /// The request never reached `Lakekeeper`, or its response could not
    /// even be read (connection refused/reset, timeout, DNS failure).
    #[error("could not reach Lakekeeper's management API")]
    Transport,
    /// `Lakekeeper` responded with a `4xx` status: OUR request was
    /// rejected (a malformed `storage-profile`, an unknown warehouse,
    /// invalid credentials against `Lakekeeper` itself, ...), not a
    /// connectivity problem. Split from [`Self::Server`] so a caller can
    /// tell "Lakekeeper is unreachable" from "Lakekeeper is reachable but
    /// refused this specific request" without inspecting a response body
    /// this type deliberately never carries — see
    /// `lakehouse_api::error::provisioning_unavailable`, this crate's one
    /// caller that acts on the distinction.
    #[error("Lakekeeper's management API rejected the request")]
    Rejected,
    /// `Lakekeeper` responded with a `5xx` status, or a `2xx` body this
    /// client could not parse into the expected shape.
    #[error("Lakekeeper's management API returned an unexpected response")]
    Server,
}

/// Classify a completed HTTP response's status into [`OpenfgaError`]'s
/// `4xx`-vs-`5xx` distinction, shared by every call site below so
/// `ensure_warehouse`/`grant_machine_principals` classify identically
/// rather than each hand-rolling its own `is_client_error()` check.
/// Returns `Ok(())` for any `2xx`.
fn classify_status(status: reqwest::StatusCode) -> Result<(), OpenfgaError> {
    if status.is_success() {
        Ok(())
    } else if status.is_client_error() {
        Err(OpenfgaError::Rejected)
    } else {
        Err(OpenfgaError::Server)
    }
}

#[derive(Debug, Deserialize)]
struct WarehouseListResponse {
    warehouses: Vec<WarehouseEntry>,
}

#[derive(Debug, Deserialize)]
struct WarehouseEntry {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
struct CreateWarehouseResponse {
    #[serde(rename = "warehouse-id")]
    warehouse_id: String,
}

/// The S3-compatible storage settings `POST /management/v1/warehouse`
/// requires to actually create a warehouse — `Lakekeeper`'s `S3Profile`
/// (verified against the `quay.io/lakekeeper/catalog:v0.13.3` image this
/// stack's `docker-compose.yml` pins: `key-prefix`, `bucket`, `region`,
/// `endpoint`, `path-style-access`, `sts-enabled`, `sts-role-arn` are its
/// serde field names) requires several fields `ensure_warehouse`'s
/// original body never sent — `{"warehouse-name", "storage-profile":
/// {"prefix"}}` — which is why every real call it made 422'd with
/// `storage-profile: missing field 'type'` and every tenant-provisioning
/// attempt failed. This struct is the caller-supplied half of the body;
/// `ensure_warehouse` owns only `storage_prefix` (computed per tenant, not
/// deployment config) and this crate's fixed `"type": "s3"`/
/// `"credential-type": "access-key"` literals `Lakekeeper` requires on
/// every S3 profile/credential.
///
/// Built by `lakehouse_api::config` from dedicated `TENANT_WAREHOUSE_S3_*`
/// settings (ADR 0002 Addendum 2: never the process's own
/// `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY`, even though a local deployment
/// may point both pairs at the same `RustFS` instance) — see
/// `lakehouse_api::routes::identity`'s tenant-provisioning module doc
/// comment for the honest-refusal behavior when a deployment has not set
/// them.
// `Debug` is safe to derive here (unlike most secret-carrying structs in
// this crate, which hand-implement it): `secret_key`'s own type
// (`Secret`) hand-implements `Debug` as a fixed redaction marker, so a
// derived `{:?}` on this struct never renders the raw value — the same
// reasoning `Secret`'s own doc comment gives for why it need not derive
// anything special itself.
#[derive(Clone, Debug)]
pub struct TenantWarehouseStorage {
    /// S3-compatible endpoint tenant warehouses are created against.
    pub endpoint: String,
    /// S3 region string sent to `Lakekeeper`.
    pub region: String,
    /// Bucket every tenant warehouse is created under (tenants are
    /// separated by `storage_prefix`/`key-prefix` within it, not by
    /// bucket).
    pub bucket: String,
    /// `true` for a path-style-addressed store (`RustFS`, most on-prem S3
    /// gateways); `false` for virtual-hosted-style (most of AWS S3 today).
    pub path_style_access: bool,
    /// Whether `Lakekeeper` should vend STS-scoped credentials for tables
    /// under this warehouse, matching `docker-compose.yml`'s
    /// `lakekeeper-warehouse-init` body.
    pub sts_enabled: bool,
    /// The role `Lakekeeper` assumes to vend those STS credentials.
    /// Meaningless when `sts_enabled` is `false`, still sent either way to
    /// mirror the init body exactly (an absent-vs-empty distinction
    /// `Lakekeeper` does not need this client to make on its behalf).
    pub sts_role_arn: String,
    /// AWS-shaped access key id for the storage credential this warehouse
    /// is created with.
    pub access_key: String,
    /// AWS-shaped secret access key — never logged, never placed in an
    /// error, only ever read via [`Secret::expose`] at the one call site
    /// that serializes it into the request body.
    pub secret_key: Secret,
}

/// A client for `Lakekeeper`'s management API, scoped to the two calls
/// `POST /api/identity/tenants` needs: find-or-create a tenant's
/// warehouse, and grant this stack's machine principals onto it. See the
/// module doc comment for why this lives in a file named `openfga.rs`.
pub struct LakekeeperAdminClient {
    base_url: String,
    admin_token: Secret,
    http: reqwest::Client,
}

impl LakekeeperAdminClient {
    /// Build a client. `base_url` is `Lakekeeper`'s own base (no trailing
    /// `/management/...` suffix), `admin_token` an admin-scoped bearer
    /// token already wrapped by the caller — see the module doc comment's
    /// "Credential handling" section for why this constructor does not
    /// read the token itself.
    #[must_use]
    pub fn new(base_url: impl Into<String>, admin_token: Secret) -> Self {
        Self {
            base_url: base_url.into(),
            admin_token,
            http: reqwest::Client::new(),
        }
    }

    /// Render the `Authorization` header value. Kept as its own method so
    /// every call site below builds the header identically and the
    /// `.expose()` escape hatch (see [`Secret`]'s doc comment) appears
    /// exactly once in this file.
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.admin_token.expose())
    }

    /// Find a warehouse named `name`, or create one with `storage_prefix`
    /// under it (and every other setting in `storage`) if none exists.
    /// Lookup-before-create is the idempotency this module is required to
    /// have: a retried/resumed tenant-provisioning call must not create a
    /// second warehouse for the same tenant. This matches
    /// `lakekeeper-authz-init`'s own `jq -r --arg name ... select(.name ==
    /// $name)` pattern in `docker-compose.yml`, copied rather than
    /// invented so this client's idempotency has the same proof the shell
    /// script's does.
    ///
    /// The create body mirrors `docker-compose.yml`'s
    /// `lakekeeper-warehouse-init` body field-for-field — see
    /// [`TenantWarehouseStorage`]'s doc comment for why that body, not the
    /// `{"prefix"}}`-only shape this method sent before, is what
    /// `Lakekeeper` actually requires. `storage_prefix` becomes
    /// `storage-profile.key-prefix`, the one field this call computes
    /// itself (per tenant) rather than takes from `storage`.
    ///
    /// # Errors
    ///
    /// [`OpenfgaError::Transport`] if a request never reaches `Lakekeeper`
    /// or its response cannot be read; [`OpenfgaError::Rejected`] if
    /// `Lakekeeper` responds `4xx` (our request was refused);
    /// [`OpenfgaError::Server`] on a `5xx` status from either call, or a
    /// `2xx` body that does not parse into the expected shape.
    pub async fn ensure_warehouse(
        &self,
        name: &str,
        storage_prefix: &str,
        storage: &TenantWarehouseStorage,
    ) -> Result<String, OpenfgaError> {
        let list_response = self
            .http
            .get(format!("{}/management/v1/warehouse", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|_| OpenfgaError::Transport)?;
        classify_status(list_response.status())?;
        let list: WarehouseListResponse = list_response
            .json()
            .await
            .map_err(|_| OpenfgaError::Server)?;
        if let Some(existing) = list.warehouses.iter().find(|w| w.name == name) {
            return Ok(existing.id.clone());
        }

        let create_response = self
            .http
            .post(format!("{}/management/v1/warehouse", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&json!({
                "warehouse-name": name,
                "storage-profile": {
                    "type": "s3",
                    "bucket": storage.bucket,
                    "region": storage.region,
                    "endpoint": storage.endpoint,
                    "path-style-access": storage.path_style_access,
                    "sts-enabled": storage.sts_enabled,
                    "sts-role-arn": storage.sts_role_arn,
                    "key-prefix": storage_prefix
                },
                "storage-credential": {
                    "type": "s3",
                    "credential-type": "access-key",
                    "aws-access-key-id": storage.access_key,
                    "aws-secret-access-key": storage.secret_key.expose()
                }
            }))
            .send()
            .await
            .map_err(|_| OpenfgaError::Transport)?;
        classify_status(create_response.status())?;
        let created: CreateWarehouseResponse = create_response
            .json()
            .await
            .map_err(|_| OpenfgaError::Server)?;
        Ok(created.warehouse_id)
    }

    /// Grant [`MACHINE_GRANTS`] onto `warehouse_id`. A `409` response (an
    /// identical write already exists — a resumed/retried provisioning
    /// run) is treated as success, matching `lakekeeper-authz-init`'s own
    /// `case "$$code" in 204|409` handling in `docker-compose.yml`
    /// verbatim.
    ///
    /// # Errors
    ///
    /// [`OpenfgaError::Transport`] if a request never reaches
    /// `Lakekeeper`; [`OpenfgaError::Server`] on any status other than
    /// `204`/`409`.
    pub async fn grant_machine_principals(&self, warehouse_id: &str) -> Result<(), OpenfgaError> {
        for (principal, verbs) in MACHINE_GRANTS {
            let writes: Vec<_> = verbs
                .iter()
                .map(|verb| json!({ "type": verb, "user": format!("oidc~{principal}") }))
                .collect();
            let response = self
                .http
                .post(format!(
                    "{}/management/v1/permissions/warehouse/{warehouse_id}/assignments",
                    self.base_url
                ))
                .header("Authorization", self.auth_header())
                .json(&json!({ "writes": writes }))
                .send()
                .await
                .map_err(|_| OpenfgaError::Transport)?;
            let status = response.status();
            if status.as_u16() != 204 && status.as_u16() != 409 {
                return Err(OpenfgaError::Server);
            }
        }
        Ok(())
    }
}
