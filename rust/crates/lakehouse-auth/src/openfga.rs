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
    /// `Lakekeeper` responded, but with a non-2xx status this client does
    /// not treat as success, or with a 2xx body this client could not
    /// parse into the expected shape.
    #[error("Lakekeeper's management API returned an unexpected response")]
    Server,
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
    /// if none exists. Lookup-before-create is the idempotency this
    /// module is required to have: a retried/resumed tenant-provisioning
    /// call must not create a second warehouse for the same tenant. This
    /// matches `lakekeeper-authz-init`'s own `jq -r --arg name ... select(.name
    /// == $name)` pattern in `docker-compose.yml`, copied rather than
    /// invented so this client's idempotency has the same proof the shell
    /// script's does.
    ///
    /// # Errors
    ///
    /// [`OpenfgaError::Transport`] if the request never reaches
    /// `Lakekeeper` or its response cannot be read;
    /// [`OpenfgaError::Server`] on a non-2xx status from either call, or a
    /// 2xx body that does not parse into the expected shape.
    pub async fn ensure_warehouse(
        &self,
        name: &str,
        storage_prefix: &str,
    ) -> Result<String, OpenfgaError> {
        let list: WarehouseListResponse = self
            .http
            .get(format!("{}/management/v1/warehouse", self.base_url))
            .header("Authorization", self.auth_header())
            .send()
            .await
            .map_err(|_| OpenfgaError::Transport)?
            .error_for_status()
            .map_err(|_| OpenfgaError::Server)?
            .json()
            .await
            .map_err(|_| OpenfgaError::Server)?;
        if let Some(existing) = list.warehouses.iter().find(|w| w.name == name) {
            return Ok(existing.id.clone());
        }

        let created: CreateWarehouseResponse = self
            .http
            .post(format!("{}/management/v1/warehouse", self.base_url))
            .header("Authorization", self.auth_header())
            .json(&json!({
                "warehouse-name": name,
                "storage-profile": { "prefix": storage_prefix }
            }))
            .send()
            .await
            .map_err(|_| OpenfgaError::Transport)?
            .error_for_status()
            .map_err(|_| OpenfgaError::Server)?
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
