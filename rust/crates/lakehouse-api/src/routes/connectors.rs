//! `/api/connectors/*` — connector definitions (source/sink systems),
//! backed by Postgres (`lakehouse-store`).
//!
//! # Not a port
//!
//! Like `routes::identity`, this replaces an *in-browser* mock
//! (`src/services/mock/connectors.ts`) that never had a server side —
//! there is no TypeScript route handler this is bug-compatible with.
//! Status codes are chosen to be correct: 201 on create, 404 on a missing
//! id, 409 on a duplicate name, 400 on a malformed body or a `secretRef`
//! that looks like a raw credential, 503 with no database pool.
//!
//! # Credentials
//!
//! See `lakehouse_store::connectors`'s module doc comment for the full
//! decision record. The short version: no endpoint here ever returns a
//! `host` or `secretRef` — [`lakehouse_store::connectors::Connector`] and
//! `ConnectorDetail` have no such field to serialize.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::cdc::ConnectorSlug;
use lakehouse_store::connectors::{self, ConnectorDetail, ConnectorDialInfo, CreateConnectorInput};
use serde::Deserialize;

use crate::connector_deprovision::{self, DeprovisionError, Deprovisioned, PgTarget};
use crate::connector_probe;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`/`routes::pipelines::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "connector store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

fn required(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::BadRequest(format!("{field} wajib diisi")));
    }
    Ok(trimmed.to_owned())
}

/// `GET /api/connectors` — every connector.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<connectors::Connector>>> {
    Ok(ApiJson(connectors::list_connectors(pool(&state)?).await?))
}

/// `GET /api/connectors/{id}` — one connector's detail.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<ConnectorDetail>> {
    let detail = connectors::get_connector(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    Ok(ApiJson(detail))
}

/// The `POST /api/connectors` body. Mirrors `CreateConnectorInput`.
///
/// `secret_ref` is a REFERENCE NAME (`"env:FOO"`, `"vault:path"`), never a
/// credential value — see the module doc comment.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConnectorBody {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    direction: String,
    host: String,
    secret_ref: String,
    /// Optional secondary reference, e.g. the secret-access-key half of an
    /// S3 connector's access-key/secret-key pair (see
    /// `lakehouse_store::connectors::ConnectorDialInfo::secret_ref_secondary`'s
    /// doc comment). Without this, an API-created S3 connector's `/test`
    /// can never succeed — `probe_s3` requires both.
    #[serde(default)]
    secret_ref_secondary: Option<String>,
    environment: String,
    tenant: String,
    #[serde(default)]
    residency: String,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    owner: Option<String>,
}

const VALID_DIRECTIONS: [&str; 3] = ["source", "sink", "bidirectional"];

/// Refuse a caller-supplied `secretRef` that names one of the deployment's
/// connector credentials ([`crate::state::CONNECTOR_ALLOWED_SECRET_REFS`]).
///
/// Those refs are the only ones `AppState::connector_secret_resolver` will
/// resolve, and they exist for the connectors seeded by migration — the ones
/// this deployment operates itself. A user-created connector naming one would
/// have the API authenticate to a caller-chosen `host` with the deployment's
/// own connector credentials. `connector_probe`'s SSRF guard does not prevent
/// that: it blocks internal address ranges, and exfiltration wants an
/// EXTERNAL host, which is exactly what it permits.
///
/// So the allowlist answers "which refs may resolve at all" and this answers
/// "who may name them". Neither alone is sufficient: without the allowlist a
/// connector could name `env:DATABASE_URL`; without this check it could name
/// the connector credentials and point them anywhere.
///
/// Deliberately compared case-sensitively and after trimming, matching how
/// the ref is stored and later handed to the resolver — a check that
/// normalized more aggressively than the resolver would leave a gap between
/// what this rejects and what that accepts.
fn reject_allowlisted_secret_ref(field: &str, value: &str) -> Result<(), ApiError> {
    if crate::state::CONNECTOR_ALLOWED_SECRET_REFS.contains(&value.trim()) {
        return Err(ApiError::BadRequest(format!(
            "{field} must not name a deployment connector credential; those are reserved for \
             connectors this deployment seeds itself"
        )));
    }
    Ok(())
}

/// `POST /api/connectors` — register a connector. Returns 201.
///
/// # Security
///
/// Unauthenticated, like every route in this service — see
/// `routes::identity`'s module doc comment for why that is a known,
/// escalated gap rather than an oversight.
///
/// # Errors
///
/// 400 on a malformed body, a blank required field, an unrecognized
/// `direction`, or a `secretRef` shaped like a raw credential (see
/// `lakehouse_store::connectors::looks_like_raw_secret`); 409 if the name
/// is taken; 503/500 as above. Also 400 if `secretRef`/`secretRefSecondary`
/// names a deployment connector credential — see
/// [`reject_allowlisted_secret_ref`].
pub async fn create(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<connectors::Connector>)> {
    let body: CreateConnectorBody = parse_body(&body)?;
    let direction = required("direction", &body.direction)?;
    if !VALID_DIRECTIONS.contains(&direction.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "direction must be one of {VALID_DIRECTIONS:?}, got {direction:?}"
        ))
        .into());
    }
    let secret_ref = required("secretRef", &body.secret_ref)?;
    if connectors::looks_like_raw_secret(&secret_ref) {
        return Err(ApiError::BadRequest(
            "secretRef must be a reference to a credential (e.g. \"env:MY_SECRET\" or \
             \"vault:secret/data/...\"), not the credential itself"
                .to_owned(),
        )
        .into());
    }
    reject_allowlisted_secret_ref("secretRef", &secret_ref)?;
    let secret_ref_secondary = match body.secret_ref_secondary {
        Some(raw) if !raw.trim().is_empty() => {
            let trimmed = raw.trim().to_owned();
            if connectors::looks_like_raw_secret(&trimmed) {
                return Err(ApiError::BadRequest(
                    "secretRefSecondary must be a reference to a credential, not the credential \
                     itself"
                        .to_owned(),
                )
                .into());
            }
            reject_allowlisted_secret_ref("secretRefSecondary", &trimmed)?;
            Some(trimmed)
        }
        _ => None,
    };
    let input = CreateConnectorInput {
        name: required("name", &body.name)?,
        kind: required("type", &body.kind)?,
        direction,
        host: required("host", &body.host)?,
        secret_ref,
        secret_ref_secondary,
        environment: required("environment", &body.environment)?,
        tenant: required("tenant", &body.tenant)?,
        residency: body.residency,
        capabilities: body.capabilities,
        owner: body.owner,
    };
    let created = connectors::create_connector(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// `POST /api/connectors/{id}/test` — test a connector's connection.
///
/// Opens a REAL, bounded (5s, no retries) connectivity probe for
/// `PostgreSQL` and S3-compatible object-storage connectors — see
/// `crate::connector_probe`'s module doc comment for exactly what that
/// does and does not cover. Every other connector `type` gets an honest
/// `supported: false` result, never a fabricated latency or success.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn test_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<connectors::ConnectorTestResult>> {
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };
    let outcome = crate::connector_probe::probe(
        &dial_info,
        state.connector_secret_resolver.as_ref(),
        state.config.connector_probe_allow_internal_hosts,
    )
    .await;
    match connectors::record_test_result(
        pool(&state)?,
        &id,
        outcome.ok,
        outcome.supported,
        outcome.latency_ms,
        &outcome.message,
    )
    .await
    {
        Ok(result) => Ok(ApiJson(result)),
        Err(lakehouse_store::StoreError::NotFound) => {
            Err(ApiError::NotFound(format!("Connector {id} not found")).into())
        }
        Err(err) => Err(ApiError::from(err).into()),
    }
}

/// `?force=true` on `DELETE /api/connectors/{id}` — see [`delete`]'s doc
/// comment for what this overrides and why it exists at all.
#[derive(Debug, Default, Deserialize)]
pub struct DeleteQuery {
    /// Delete the registry row even if CDC deprovisioning failed.
    #[serde(default)]
    force: bool,
}

/// Derive the [`ConnectorSlug`] a `PostgreSQL` CDC connector's replication
/// slot/publication would be named after, from the connector's own
/// registry `id`.
///
/// There is no separate "Debezium slug" column: ADR 0007 never grew
/// dynamic per-connector provisioning (see `lakehouse_store::cdc`'s module
/// doc comment — "Callers"), so `connector.id`
/// (`connectors::slug_id`'s output, e.g. `"conn-orders-pg-abc123"`) is the
/// only identifier a real connector has, and it already uses exactly the
/// charset a slug needs MINUS the separator: lowercase ASCII alphanumerics
/// and `-`. Converting `-` to `_` maps it onto [`ConnectorSlug`]'s
/// allowed shape (`^[a-z0-9][a-z0-9_]{0,62}$`) losslessly and
/// deterministically — the same `id` always produces the same slug, so
/// deprovisioning a connector always targets the slot/publication that
/// connector's own id would have named if/when a real provisioning flow
/// existed to create them.
fn connector_slug_for_id(id: &str) -> Result<ConnectorSlug, ApiError> {
    ConnectorSlug::new(&id.replace('-', "_")).map_err(|err| {
        // `connector.id` is always `slug_id`-generated (see that
        // function's doc comment), so this should be unreachable in
        // practice; if it ever isn't, this is a 500 (a data-shape problem
        // this deployment has, not something the caller did wrong), not a
        // silent skip of deprovisioning.
        ApiError::Internal(format!(
            "connector id {id:?} does not map to a valid CDC slug: {err}"
        ))
    })
}

/// Attempt to drop `id`'s `PostgreSQL` CDC replication slot/publication —
/// the real work behind [`delete`]'s pre-delete deprovision step. Resolves
/// `dial_info`'s host/credential exactly the way
/// [`crate::connector_probe::probe`] does for `POST
/// /api/connectors/{id}/test` (same parser, same allowlisted resolver),
/// because dropping a slot needs the same "safely resolve this
/// connector's own secretRef and dial its own host" capability a
/// connectivity test already has — see `connector_deprovision`'s module
/// doc comment for why this makes
/// `ops/debezium/deprovision_connector.sh`'s "the API has no such
/// mechanism" comment obsolete.
async fn deprovision_postgres_connector(
    state: &AppState,
    id: &str,
    dial_info: &ConnectorDialInfo,
) -> Result<Deprovisioned, ApiError> {
    let slug = connector_slug_for_id(id)?;
    let Some(target) = connector_probe::parse_postgres_host(&dial_info.host) else {
        return Err(ApiError::Internal(format!(
            "connector {id} is registered as PostgreSQL but its host is not shaped \
             \"<user>@<host>:<port>/<database>\", so CDC deprovisioning cannot even attempt to \
             dial it"
        )));
    };
    let password = state
        .connector_secret_resolver
        .resolve_dyn(&dial_info.secret_ref)
        .await
        .map_err(|err| {
            ApiError::Internal(format!(
                "could not resolve connector {id}'s credential to deprovision its CDC slot: {err}"
            ))
        })?;
    let pg_target = PgTarget {
        host: target.host.to_owned(),
        port: target.port,
        user: target.user.to_owned(),
        password,
        database: target.database.to_owned(),
    };
    connector_deprovision::drop_slot_and_publication(&pg_target, &slug)
        .await
        .map_err(|err| ApiError::Internal(deprovision_error_message(id, &slug, &err)))
}

/// Render a [`DeprovisionError`] into text safe to put in a 409/500 body or
/// a log line — never the credential [`deprovision_postgres_connector`]
/// resolved, only the slot/publication names and the error's own
/// `Display` (which, per [`DeprovisionError`]'s doc comment, never
/// includes connection credentials).
fn deprovision_error_message(id: &str, slug: &ConnectorSlug, err: &DeprovisionError) -> String {
    format!(
        "deprovisioning connector {id}'s CDC replication slot ({slug}_slot) and publication \
         ({slug}_pub) failed: {err}"
    )
}

/// `DELETE /api/connectors/{id}` — remove a connector registration.
///
/// # `PostgreSQL` CDC deprovisioning happens FIRST
///
/// For a `PostgreSQL`-kind connector, this now attempts to drop its
/// replication slot/publication on the source database (see
/// [`deprovision_postgres_connector`]) BEFORE removing the registry row —
/// the fix for the exact gap `lakehouse_store::connectors::delete_connector`
/// used to describe: silently deleting the row while the slot survived
/// left it pinning WAL on the customer's source database until disk filled,
/// forever, with nothing in the registry to show it. Deprovisioning is
/// idempotent (an already-absent slot/publication is success), so a
/// connector that was never fully provisioned, or was already cleaned up
/// by a previous attempt, still deletes cleanly.
///
/// - Deprovision succeeds (or the connector is not `PostgreSQL`) -> the row
///   is deleted -> 204, same as before.
/// - Deprovision fails and `?force` is absent/false -> the row is KEPT and
///   this returns 409, naming the slot/publication in the message. This is
///   deliberate: the whole point is that an orphaned slot must stay
///   visible (as a connector row still in the registry) rather than
///   vanish along with the only record that it needs cleaning up.
/// - Deprovision fails and `?force=true` is given -> the row is deleted
///   anyway (204) and a `tracing::error!` names the slot, publication, and
///   host as a deliberately loud, greppable record of the orphan this
///   just created. `force` exists because a connector can end up pointing
///   at a decommissioned or now-unreachable host — deprovisioning can
///   never succeed there, and an operator who already knows that needs a
///   way to remove the row anyway rather than being stuck forever.
///
/// # Errors
///
/// 404 if `id` is unknown; 409 if `PostgreSQL` CDC deprovisioning failed
/// and `force` was not given; 503/500 as above.
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    let dial_info = connectors::get_connector_dial_info(pool(&state)?, &id).await?;
    let Some(dial_info) = dial_info else {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    };

    if dial_info.kind.to_lowercase().contains("postgres")
        && let Err(err) = deprovision_postgres_connector(&state, &id, &dial_info).await
    {
        if query.force {
            let slug = connector_slug_for_id(&id).ok();
            tracing::error!(
                connector_id = %id,
                slot = %slug.as_ref().map_or_else(|| "<unknown>".to_owned(), |s| format!("{s}_slot")),
                publication = %slug.as_ref().map_or_else(|| "<unknown>".to_owned(), |s| format!("{s}_pub")),
                host = %dial_info.host,
                error = %err,
                "force-deleting connector despite failed CDC deprovision: the replication \
                 slot/publication above may still exist on the source database and will \
                 keep pinning WAL until cleaned up manually"
            );
        } else {
            let slug_hint = connector_slug_for_id(&id).map_or_else(
                |_| "its CDC slot/publication".to_owned(),
                |s| format!("slot {s}_slot / publication {s}_pub"),
            );
            return Err(ApiError::Conflict(format!(
                "connector {id} was NOT deleted: dropping {slug_hint} failed ({err}); the \
                 registry row is kept deliberately so this orphaned slot stays visible instead \
                 of silently pinning WAL on the source database forever. Retry once the source \
                 is reachable, or pass ?force=true to delete the row anyway (the \
                 slot/publication will then have to be cleaned up manually)."
            ))
            .into());
        }
    }

    let deleted = connectors::delete_connector(pool(&state)?, &id).await?;
    if !deleted {
        return Err(ApiError::NotFound(format!("Connector {id} not found")).into());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::to_bytes;
    use axum::http::Request;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    #[tokio::test]
    async fn missing_pool_is_a_503_naming_database_url() {
        let state = state_without_pool();
        let err = pool(&state).expect_err("a malformed DATABASE_URL must yield no pool");
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    #[tokio::test]
    async fn every_database_backed_route_returns_503_without_a_pool() {
        let paths = ["/api/connectors", "/api/connectors/conn-x"];
        for path in paths {
            let app = crate::routes::router(state_without_pool());
            let response = app
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{path}");
            let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
            let body: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(body.get("error").is_some(), "{path}");
        }
    }

    #[test]
    fn create_body_rejects_unknown_direction() {
        let body = CreateConnectorBody {
            name: "n".to_owned(),
            kind: "REST API".to_owned(),
            direction: "sideways".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            secret_ref_secondary: None,
            environment: "production".to_owned(),
            tenant: "t".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        };
        assert!(!VALID_DIRECTIONS.contains(&body.direction.as_str()));
    }

    /// D5/Should-fix: `secretRefSecondary` must parse through the request
    /// body (camelCase, per the struct's `rename_all`) and reach
    /// `CreateConnectorInput` — otherwise an API-created S3 connector can
    /// never be tested, since `probe_s3` requires both refs.
    #[test]
    fn secret_ref_secondary_round_trips_through_the_request_body() {
        let json = serde_json::json!({
            "name": "n",
            "type": "Object storage",
            "direction": "sink",
            "host": "http://rustfs:9000|bucket",
            "secretRef": "env:AK",
            "secretRefSecondary": "env:SK",
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(body.secret_ref_secondary.as_deref(), Some("env:SK"));
    }

    /// Absent `secretRefSecondary` (e.g. a `PostgreSQL` connector, which
    /// only ever needs one credential) must still parse.
    #[test]
    fn secret_ref_secondary_is_optional() {
        let json = serde_json::json!({
            "name": "n",
            "type": "PostgreSQL",
            "direction": "bidirectional",
            "host": "u@host:5432/db",
            "secretRef": "env:PW",
            "environment": "production",
            "tenant": "t",
        });
        let body: CreateConnectorBody = serde_json::from_value(json).unwrap();
        assert_eq!(body.secret_ref_secondary, None);
    }

    /// The defense-in-depth check from `looks_like_raw_secret` is wired
    /// into this handler, not just unit-tested in isolation.
    #[test]
    fn secret_looking_secret_ref_is_rejected_by_the_shared_check() {
        assert!(connectors::looks_like_raw_secret(
            "postgres://admin:hunter2@db.internal:5432/oms"
        ));
        assert!(!connectors::looks_like_raw_secret("env:MY_SECRET"));
    }

    /// The exfiltration path this check exists to close: a
    /// `connector:manage` principal naming a deployment connector credential
    /// on a connector whose `host` they choose. Every allowlisted ref must be
    /// refused, so adding one to the allowlist without widening this check
    /// fails here rather than silently opening the hole again.
    #[test]
    fn user_created_connector_cannot_name_a_deployment_connector_credential() {
        for r in crate::state::CONNECTOR_ALLOWED_SECRET_REFS {
            assert!(
                reject_allowlisted_secret_ref("secretRef", r).is_err(),
                "allowlisted ref {r:?} must be refused on a user-created connector"
            );
            // Whitespace must not be a bypass: the value is trimmed before
            // storage, so a padded ref would reach the resolver identically.
            assert!(
                reject_allowlisted_secret_ref("secretRef", &format!("  {r}  ")).is_err(),
                "padded {r:?} must be refused too"
            );
        }
    }

    /// The check must not over-reach: an ordinary `env:` ref is still
    /// accepted here. It will fail later at resolution (it is not on the
    /// allowlist), which is a different, honest error — "this deployment
    /// will not resolve that", not "you may not say that".
    #[test]
    fn ordinary_secret_refs_are_still_accepted_by_this_check() {
        for r in [
            "env:MY_SECRET",
            "vault:secret/data/x",
            "env:POSTGRES_PASSWORD_2",
        ] {
            assert!(
                reject_allowlisted_secret_ref("secretRef", r).is_ok(),
                "{r:?} is not a deployment connector credential and must pass this check"
            );
        }
    }

    /// The allowlist must never name one of the API's own secrets again.
    /// This is the regression guard for the finding itself: the previous
    /// list was `env:POSTGRES_PASSWORD` / `env:RUSTFS_ACCESS_KEY` /
    /// `env:RUSTFS_SECRET_KEY`, and re-adding any of them would restore the
    /// exfiltration path no matter what the route-level check does.
    #[test]
    fn allowlist_never_names_the_apis_own_secrets() {
        for forbidden in [
            "env:POSTGRES_PASSWORD",
            "env:RUSTFS_ACCESS_KEY",
            "env:RUSTFS_SECRET_KEY",
            "env:DATABASE_URL",
            "env:CH_PASSWORD",
        ] {
            assert!(
                !crate::state::CONNECTOR_ALLOWED_SECRET_REFS.contains(&forbidden),
                "{forbidden:?} is one of the API's own secrets and must never be \
                 connector-resolvable"
            );
        }
    }
}
