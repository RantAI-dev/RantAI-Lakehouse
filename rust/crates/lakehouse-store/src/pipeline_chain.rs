//! The stored outcomes of a re-upload's downstream pipeline chain (`BLD-14`,
//! `DEC-22`, D8; migration `0153_pipeline_chain_step.sql` -- read its header).
//!
//! A chain sensor reports one outcome per (re-upload, pipeline) through the
//! orchestrator-only route; [`record_chain_step`] writes it once and keeps the
//! newest 20 per pipeline (P9). [`latest_chain_step`] is what the pipeline page
//! reads to say why a pipeline did or did not run. This module stores codes and
//! ids only: the orchestrator's and the engine's own text never enters a row
//! (`AGENTS.md` principle 4). It does not decide when a pipeline runs -- that
//! is the Dagster chain sensor's job.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// One outcome reported by a chain sensor for a pipeline of a re-upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainStep {
    /// The `file_ingest_job` run that loaded the bronze table.
    pub root_run_id: String,
    /// The pipeline whose outcome this is.
    pub pipeline_id: String,
    /// The `bronze.<table>` the re-upload loaded.
    pub source_table: String,
    /// `started`, `not_run` or `superseded`.
    pub outcome: String,
    /// `upstream_failed`, `upstream_cancelled` or `wait_expired` for `not_run`.
    pub reason: Option<String>,
    /// The started run's id; present only for `started`.
    pub run_id: Option<String>,
    /// The pipeline that stopped the chain; present only for `not_run`.
    pub cause_pipeline_id: Option<String>,
}

/// The newest outcome of a pipeline, with the cause's name when the cause is
/// in the same tenant. Serde uses camelCase to mirror the API's JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct ChainStepView {
    /// The `file_ingest_job` run that loaded the bronze table.
    pub root_run_id: String,
    /// The `bronze.<table>` the re-upload loaded.
    pub source_table: String,
    /// `started`, `not_run` or `superseded`.
    pub outcome: String,
    /// The reason for `not_run`; null otherwise.
    pub reason: Option<String>,
    /// The started run's id; null otherwise.
    pub run_id: Option<String>,
    /// The pipeline that stopped the chain; null otherwise.
    pub cause_pipeline_id: Option<String>,
    /// The cause's name, null when it was deleted or is in another tenant.
    pub cause_pipeline_name: Option<String>,
    /// When the step was recorded.
    pub recorded_at: OffsetDateTime,
}

/// Record one chain outcome. The pipeline's tenant is copied from
/// `pipeline_definition` in the insert's own `SELECT`, so a row can never name
/// a tenant the pipeline did not have at insert time. A second write for the
/// same (root, pipeline) is a no-op: `true` is returned only for the first.
/// The insert and P9's "all but the newest 20" sweep share one transaction.
///
/// # Errors
///
/// [`StoreError::NotFound`] when `pipeline_id` has no definition.
/// [`StoreError::Database`] on any query or transaction failure.
pub async fn record_chain_step(pool: &PgPool, step: &ChainStep) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let inserted = sqlx::query_scalar::<_, String>(
        "INSERT INTO pipeline_chain_step \
           (root_run_id, pipeline_id, tenant_id, source_table, outcome, reason, \
            run_id, cause_pipeline_id) \
         SELECT $1, $2, tenant_id, $3, $4, $5, $6, $7 \
           FROM pipeline_definition WHERE id = $2 \
         ON CONFLICT (root_run_id, pipeline_id) DO NOTHING \
         RETURNING root_run_id",
    )
    .bind(&step.root_run_id)
    .bind(&step.pipeline_id)
    .bind(&step.source_table)
    .bind(&step.outcome)
    .bind(&step.reason)
    .bind(&step.run_id)
    .bind(&step.cause_pipeline_id)
    .fetch_optional(&mut *tx)
    .await?
    .is_some();
    if !inserted {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pipeline_definition WHERE id = $1)")
                .bind(&step.pipeline_id)
                .fetch_one(&mut *tx)
                .await?;
        if !exists {
            return Err(StoreError::NotFound);
        }
    }
    // P9: the newest 20 rows of this pipeline survive. `ctid` identifies the
    // rows to keep so two rows with the same `recorded_at` cannot both survive
    // a subquery keyed on the timestamp alone.
    sqlx::query(
        "DELETE FROM pipeline_chain_step WHERE pipeline_id = $1 AND ctid IN ( \
           SELECT ctid FROM pipeline_chain_step WHERE pipeline_id = $1 \
           ORDER BY recorded_at DESC OFFSET 20 \
         )",
    )
    .bind(&step.pipeline_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(inserted)
}

/// Read the newest chain outcome for a pipeline. The cause's name is joined
/// only when the cause is in the same tenant (section 3); `IS NOT DISTINCT`
/// makes a shared row (tenant null) visible to a shared cause and hides it
/// from a tenant-owned one.
///
/// # Errors
///
/// [`StoreError::Database`] on any query failure.
pub async fn latest_chain_step(
    pool: &PgPool,
    pipeline_id: &str,
) -> Result<Option<ChainStepView>, StoreError> {
    let row = sqlx::query_as::<_, ChainStepView>(
        "SELECT s.root_run_id, s.source_table, s.outcome, s.reason, s.run_id, \
                s.cause_pipeline_id, c.name AS cause_pipeline_name, s.recorded_at \
           FROM pipeline_chain_step s \
           LEFT JOIN pipeline_definition c \
             ON c.id = s.cause_pipeline_id \
            AND c.tenant_id IS NOT DISTINCT FROM s.tenant_id \
          WHERE s.pipeline_id = $1 \
          ORDER BY s.recorded_at DESC LIMIT 1",
    )
    .bind(pipeline_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}
