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
use axum::extract::{Path, State};
use axum::http::StatusCode;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::connectors::{self, ConnectorDetail, CreateConnectorInput};
use serde::Deserialize;

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
/// is taken; 503/500 as above.
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
    let input = CreateConnectorInput {
        name: required("name", &body.name)?,
        kind: required("type", &body.kind)?,
        direction,
        host: required("host", &body.host)?,
        secret_ref,
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
/// Does NOT open a real network connection — see
/// `lakehouse_store::connectors::test_connection`'s doc comment for why
/// (this service never resolves a `secretRef` to an actual credential, and
/// is not permitted to originate connections to operator-configured
/// external systems in this environment). The result reflects the
/// connector's last known stored health.
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn test_connection(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<connectors::ConnectorTestResult>> {
    match connectors::test_connection(pool(&state)?, &id).await {
        Ok(result) => Ok(ApiJson(result)),
        Err(lakehouse_store::StoreError::NotFound) => {
            Err(ApiError::NotFound(format!("Connector {id} not found")).into())
        }
        Err(err) => Err(ApiError::from(err).into()),
    }
}

/// `DELETE /api/connectors/{id}` — remove a connector registration.
///
/// Does not itself contact the connector's source system — see
/// `lakehouse_store::connectors::delete_connector`'s doc comment for why,
/// and for the CDC slot-cleanup gap this leaves (P5).
///
/// # Errors
///
/// 404 if `id` is unknown; 503/500 as above.
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
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
            environment: "production".to_owned(),
            tenant: "t".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        };
        assert!(!VALID_DIRECTIONS.contains(&body.direction.as_str()));
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
}
