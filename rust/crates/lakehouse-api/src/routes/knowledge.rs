//! `/api/knowledge/*` — knowledge sources and vector jobs, backed by
//! Postgres (`lakehouse-store`).
//!
//! # Not a port
//!
//! Like `routes::identity`/`routes::connectors`, this replaces an
//! *in-browser* mock (`src/services/mock/knowledge.ts`) that never had a
//! server side — there is no TypeScript route handler this is
//! bug-compatible with. Status codes are chosen to be correct: 201 on
//! create, 409 on a duplicate name, 400 on a malformed body, 503 with no
//! database pool.
//!
//! # `semanticSearch` is NOT here
//!
//! There is no vector database, embedding engine, or search index in this
//! deployment (see `lakehouse_store::knowledge`'s module doc comment for
//! the investigation). This module intentionally has no search handler:
//! `KnowledgeService::search` (`src/services/clients/knowledge.ts`) keeps
//! delegating to `mockKnowledgeService.search` rather than this crate
//! fabricating similarity scores against content that isn't actually
//! indexed anywhere.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::knowledge::{
    self, CreateSourceInput, CreateVectorJobInput, KnowledgeSource, VectorJob,
};
use serde::Deserialize;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`/`routes::connectors::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "knowledge store unavailable: no Postgres pool is configured \
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

const VALID_KINDS: [&str; 6] = ["file", "object-storage", "web", "table", "query", "manual"];
const VALID_CLASSIFICATIONS: [&str; 4] = ["public", "internal", "confidential", "restricted"];

/// `GET /api/knowledge/sources` — every knowledge source.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_sources(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<KnowledgeSource>>> {
    Ok(ApiJson(knowledge::list_sources(pool(&state)?).await?))
}

/// `GET /api/knowledge/vector-jobs` — every vector job.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_vector_jobs(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<VectorJob>>> {
    Ok(ApiJson(knowledge::list_vector_jobs(pool(&state)?).await?))
}

/// The `POST /api/knowledge/sources` body. Mirrors `CreateKnowledgeSourceInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSourceBody {
    name: String,
    kind: String,
    embedding_model: String,
    classification: String,
    #[serde(default)]
    owner: Option<String>,
}

/// `POST /api/knowledge/sources` — register a knowledge source. Returns
/// 201.
///
/// # Errors
///
/// 400 on a malformed body, a blank required field, or an unrecognized
/// `kind`/`classification`; 409 if the name is taken; 503/500 as above.
pub async fn create_source(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<KnowledgeSource>)> {
    let body: CreateSourceBody = parse_body(&body)?;
    let kind = required("kind", &body.kind)?;
    if !VALID_KINDS.contains(&kind.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "kind must be one of {VALID_KINDS:?}, got {kind:?}"
        ))
        .into());
    }
    let classification = required("classification", &body.classification)?;
    if !VALID_CLASSIFICATIONS.contains(&classification.as_str()) {
        return Err(ApiError::BadRequest(format!(
            "classification must be one of {VALID_CLASSIFICATIONS:?}, got {classification:?}"
        ))
        .into());
    }
    let input = CreateSourceInput {
        name: required("name", &body.name)?,
        kind,
        embedding_model: required("embeddingModel", &body.embedding_model)?,
        classification,
        owner: body.owner,
    };
    let created = knowledge::create_source(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// The `POST /api/knowledge/vector-jobs` body. Mirrors `CreateVectorJobInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateVectorJobBody {
    name: String,
    source: String,
    embedding_model: String,
    index_type: String,
    #[serde(default)]
    owner: Option<String>,
}

/// `POST /api/knowledge/vector-jobs` — create a vector job. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body or a blank required field; 409 if the name is
/// taken; 503/500 as above.
pub async fn create_vector_job(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<VectorJob>)> {
    let body: CreateVectorJobBody = parse_body(&body)?;
    let input = CreateVectorJobInput {
        name: required("name", &body.name)?,
        source: required("source", &body.source)?,
        embedding_model: required("embeddingModel", &body.embedding_model)?,
        index_type: required("indexType", &body.index_type)?,
        owner: body.owner,
    };
    let created = knowledge::create_vector_job(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
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
    async fn every_database_backed_route_returns_503_without_a_pool() {
        let paths = ["/api/knowledge/sources", "/api/knowledge/vector-jobs"];
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
    fn create_source_body_rejects_unknown_kind() {
        let body = CreateSourceBody {
            name: "n".to_owned(),
            kind: "carrier-pigeon".to_owned(),
            embedding_model: "m".to_owned(),
            classification: "internal".to_owned(),
            owner: None,
        };
        assert!(!VALID_KINDS.contains(&body.kind.as_str()));
    }
}
