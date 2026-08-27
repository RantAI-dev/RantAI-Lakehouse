//! Repository layer for the `knowledge` domain: declared knowledge sources
//! and the vector jobs that index them. Postgres backing for
//! `src/services/mock/knowledge.ts`.
//!
//! # What this module does NOT do
//!
//! There is no vector database, embedding engine, or search index anywhere
//! in this repository or the real infrastructure this deployment points
//! at — see `0015_knowledge.sql`'s header comment for the investigation
//! that established this (no `Array(Float32|Float64)` embedding column or
//! vector index exists in the live `ClickHouse` instance). This module
//! therefore only stores and serves *metadata*: what sources are
//! registered, what vector jobs exist and their status. It has no
//! `search`/`semantic_search` function — [`crate`]'s callers must keep
//! `KnowledgeService::search` delegating to the mock (see
//! `src/services/clients/knowledge.ts`), because fabricating similarity
//! scores against a document store that does not exist would be strictly
//! worse than an honestly-labeled mock.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

/// Mirrors `KnowledgeSource` in `contracts/knowledge.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSource {
    /// `knowledge_source.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// `"file" | "object-storage" | "web" | "table" | "query" | "manual"`.
    pub kind: String,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Owning team or person.
    pub owner: String,
    /// Version label.
    pub version: String,
    /// Last refresh time.
    #[sqlx(rename = "last_refresh")]
    #[serde(rename = "lastRefresh", serialize_with = "ser_ts")]
    pub last_refresh: OffsetDateTime,
    /// Number of indexed chunks.
    pub chunk_count: i64,
    /// Embedding model label (metadata only — see the module doc comment).
    pub embedding_model: String,
    /// `"ready" | "indexing" | "degraded"`.
    pub index_status: String,
    /// Freshness lag, in seconds.
    pub freshness_lag_seconds: i64,
    /// Data classification level.
    pub classification: String,
    /// Number of agents that depend on this source.
    pub dependent_agents: i64,
    /// Catalog asset id for this corpus, when registered.
    pub asset_id: Option<String>,
    /// Active vector job producing embeddings for this source.
    pub vector_job_id: Option<String>,
}

fn ser_ts<S: serde::Serializer>(at: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&iso_millis(*at))
}

/// Mirrors `VectorJob` in `contracts/knowledge.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct VectorJob {
    /// `vector_job.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// Lifecycle status (`EntityStatus`).
    pub status: String,
    /// Source display label (free text — see `source_id` for the FK).
    pub source: String,
    /// `knowledge_source.id` this job indexes, when it names a registered
    /// source.
    pub source_id: Option<String>,
    /// Resulting vector / knowledge asset in the catalog.
    pub output_asset_id: Option<String>,
    /// Embedding model label (metadata only — see the module doc comment).
    pub embedding_model: String,
    /// Index type label (e.g. `"HNSW"`).
    pub index_type: String,
    /// Last run time.
    #[sqlx(rename = "last_run_at")]
    #[serde(rename = "lastRunAt", serialize_with = "ser_ts")]
    pub last_run_at: OffsetDateTime,
    /// Owning team or person.
    pub owner: String,
}

const SOURCE_COLUMNS: &str = "id, name, kind, status, owner, version, last_refresh, chunk_count, \
     embedding_model, index_status, freshness_lag_seconds, classification, dependent_agents, \
     asset_id, vector_job_id";

const JOB_COLUMNS: &str = "id, name, status, source, source_id, output_asset_id, \
     embedding_model, index_type, last_run_at, owner";

/// List every knowledge source, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_sources(pool: &PgPool) -> Result<Vec<KnowledgeSource>, StoreError> {
    let sql = format!("SELECT {SOURCE_COLUMNS} FROM knowledge_source ORDER BY created_at DESC");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// List every vector job, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_vector_jobs(pool: &PgPool) -> Result<Vec<VectorJob>, StoreError> {
    let sql = format!("SELECT {JOB_COLUMNS} FROM vector_job ORDER BY created_at DESC");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// Everything [`create_source`] needs. Mirrors `CreateKnowledgeSourceInput`.
#[derive(Debug, Clone)]
pub struct CreateSourceInput {
    /// Display name; must not collide with an existing source.
    pub name: String,
    /// `"file" | "object-storage" | "web" | "table" | "query" | "manual"`.
    pub kind: String,
    /// Embedding model label (metadata only — see the module doc comment).
    pub embedding_model: String,
    /// Data classification level.
    pub classification: String,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

const DEFAULT_OWNER: &str = "Current user";

/// Register a knowledge source. `status` starts `"draft"`, `indexStatus`
/// starts `"indexing"`, `chunkCount`/`dependentAgents`/`freshnessLagSeconds`
/// start at `0` — same as `mock/knowledge.ts`'s `createSource`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken.
pub async fn create_source(
    pool: &PgPool,
    input: &CreateSourceInput,
) -> Result<KnowledgeSource, StoreError> {
    let id = slug_id("ks", &input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let sql = format!(
        "INSERT INTO knowledge_source (id, name, kind, status, owner, version, embedding_model, \
         index_status, classification) \
         VALUES ($1, $2, $3, 'draft', $4, 'v1', $5, 'indexing', $6) \
         RETURNING {SOURCE_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(owner)
        .bind(&input.embedding_model)
        .bind(&input.classification)
        .fetch_one(pool)
        .await?)
}

/// Everything [`create_vector_job`] needs. Mirrors `CreateVectorJobInput`.
#[derive(Debug, Clone)]
pub struct CreateVectorJobInput {
    /// Display name; must not collide with an existing job.
    pub name: String,
    /// Source display label; matched against `knowledge_source.name` to
    /// resolve `sourceId`.
    pub source: String,
    /// Embedding model label (metadata only — see the module doc comment).
    pub embedding_model: String,
    /// Index type label (e.g. `"HNSW"`).
    pub index_type: String,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

/// Create a vector job. `status` starts `"draft"` — same as
/// `mock/knowledge.ts`'s `createVectorJob`. `sourceId` is resolved by
/// matching `source` against a registered `knowledge_source.name`; left
/// `NULL` when there is no match, matching the mock's optional field.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken.
pub async fn create_vector_job(
    pool: &PgPool,
    input: &CreateVectorJobInput,
) -> Result<VectorJob, StoreError> {
    let id = slug_id("vj", &input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let source_id: Option<String> =
        sqlx::query_scalar("SELECT id FROM knowledge_source WHERE name = $1")
            .bind(&input.source)
            .fetch_optional(pool)
            .await?;
    let sql = format!(
        "INSERT INTO vector_job (id, name, status, source, source_id, embedding_model, \
         index_type, owner) \
         VALUES ($1, $2, 'draft', $3, $4, $5, $6, $7) \
         RETURNING {JOB_COLUMNS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.source)
        .bind(&source_id)
        .bind(&input.embedding_model)
        .bind(&input.index_type)
        .bind(owner)
        .fetch_one(pool)
        .await?)
}

/// A slug-based id, same shape `connectors::slug_id`/`pipelines::slug_id`
/// use (`"<prefix>-<slug>-<base36 millis>"`).
fn slug_id(prefix: &str, name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(32).collect();
    let slug = slug.trim_matches('-');
    let millis = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    #[allow(
        clippy::cast_sign_loss,
        reason = "unix millis since epoch is always positive"
    )]
    let millis = millis as u128;
    format!(
        "{prefix}-{}-{}",
        if slug.is_empty() { "new" } else { slug },
        radix36(millis)
    )
}

/// Render `n` in base 36 lowercase, matching JavaScript's
/// `n.toString(36)`. Duplicated from `connectors::radix36`/
/// `pipelines::radix36` — both private to their module, and the
/// duplication is three lines.
fn radix36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn slug_id_uses_prefix_and_lowercases() {
        let id = slug_id("ks", "Order Events Stream!!");
        assert!(id.starts_with("ks-order-events-stream-"));
    }

    #[test]
    fn radix36_matches_js_to_string_36() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
    }
}
