//! Repository layer for authored pipeline definitions: the Postgres backing
//! for `createPipeline`, `generatePipelineFromPrompt`, and the "draft" half
//! of `pausePipeline`/`resumePipeline` — pipelines a console user declared
//! that no Dagster job (yet) implements.
//!
//! See `0007_pipelines.sql`'s header comment for the full "what this is /
//! is not" reasoning: `GET /api/pipelines` stays Dagster-backed for real
//! job data, and unions this table's rows on top so a freshly authored
//! pipeline is visible immediately rather than vanishing the way an
//! authored governance rule did before the Task 2.3 gap fix.

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

/// An authored pipeline. Mirrors `Pipeline` in `contracts/pipelines.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pipeline {
    /// `pipeline_definition.id`.
    pub id: String,
    /// Pipeline name; the table's natural key.
    pub name: String,
    /// `"batch" | "incremental" | "document" | "vector"`.
    pub kind: String,
    /// Lifecycle status; `"draft"` for a freshly authored pipeline.
    pub status: String,
    /// Who owns this pipeline.
    pub owner: String,
    /// Source location (e.g. `"zone.table"`).
    pub source: String,
    /// Target location.
    pub target: String,
    /// Ingress connector id, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_id: Option<String>,
    /// Source catalog asset id, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_asset_id: Option<String>,
    /// Target catalog asset id, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_asset_id: Option<String>,
    /// Schedule label.
    pub schedule: String,
    /// Last run time, ISO 8601.
    pub last_run_at: String,
    /// Next scheduled run time, ISO 8601, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    /// Whether the pipeline is currently meeting its SLA.
    pub sla_ok: bool,
    /// Current freshness lag in seconds.
    pub freshness_lag_seconds: i32,
}

#[derive(Debug, FromRow)]
struct PipelineRow {
    id: String,
    name: String,
    kind: String,
    status: String,
    owner: String,
    source: String,
    target: String,
    connector_id: Option<String>,
    source_asset_id: Option<String>,
    target_asset_id: Option<String>,
    schedule: String,
    last_run_at: OffsetDateTime,
    next_run_at: Option<OffsetDateTime>,
    sla_ok: bool,
    freshness_lag_seconds: i32,
}

impl From<PipelineRow> for Pipeline {
    fn from(row: PipelineRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            kind: row.kind,
            status: row.status,
            owner: row.owner,
            source: row.source,
            target: row.target,
            connector_id: row.connector_id,
            source_asset_id: row.source_asset_id,
            target_asset_id: row.target_asset_id,
            schedule: row.schedule,
            last_run_at: iso_millis(row.last_run_at),
            next_run_at: row.next_run_at.map(iso_millis),
            sla_ok: row.sla_ok,
            freshness_lag_seconds: row.freshness_lag_seconds,
        }
    }
}

const PIPELINE_COLUMNS: &str = "id, name, kind, status, owner, source, target, connector_id, \
     source_asset_id, target_asset_id, schedule, last_run_at, next_run_at, sla_ok, \
     freshness_lag_seconds";

/// List every authored pipeline definition, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_pipelines(pool: &PgPool) -> Result<Vec<Pipeline>, StoreError> {
    let sql =
        format!("SELECT {PIPELINE_COLUMNS} FROM pipeline_definition ORDER BY created_at DESC");
    let rows: Vec<PipelineRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Pipeline::from).collect())
}

/// Fetch one authored pipeline by id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_pipeline(pool: &PgPool, id: &str) -> Result<Option<Pipeline>, StoreError> {
    let sql = format!("SELECT {PIPELINE_COLUMNS} FROM pipeline_definition WHERE id = $1");
    let row: Option<PipelineRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(Pipeline::from))
}

/// A slug-based id in the same shape `mock/pipelines.ts`'s `slugId` used
/// (`"pl-<slug>-<base36 millis>"`), so ids created by this store don't
/// collide with a Dagster job name (which is never prefixed `pl-`) or with
/// each other.
fn slug_id(name: &str) -> String {
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
        "pl-{}-{}",
        if slug.is_empty() { "new" } else { slug },
        radix36(millis)
    )
}

/// Render `n` in base 36 lowercase, matching JavaScript's
/// `n.toString(36)`.
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

/// Everything [`create_pipeline`] needs. Mirrors `CreatePipelineInput`.
#[derive(Debug, Clone)]
pub struct CreatePipelineInput {
    /// Pipeline name; must not collide with an existing pipeline.
    pub name: String,
    /// Pipeline kind.
    pub kind: String,
    /// Source zone (e.g. `"bronze"`).
    pub source_zone: String,
    /// Source table.
    pub source_table: String,
    /// Target zone.
    pub target_zone: String,
    /// Target table.
    pub target_table: String,
    /// Schedule label.
    pub schedule: String,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

const DEFAULT_OWNER: &str = "Current user";

/// Create an authored pipeline, matching `mock/pipelines.ts`'s
/// `fromCreateInput`: always starts `status: "draft"`, `slaOk: true`,
/// `freshnessLagSeconds: 0`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_pipeline(
    pool: &PgPool,
    input: &CreatePipelineInput,
) -> Result<Pipeline, StoreError> {
    let id = slug_id(&input.name);
    let source = format!("{}.{}", input.source_zone, input.source_table);
    let target = format!("{}.{}", input.target_zone, input.target_table);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let sql = format!(
        "INSERT INTO pipeline_definition (id, name, kind, status, owner, source, target, \
         schedule, sla_ok, freshness_lag_seconds) \
         VALUES ($1, $2, $3, 'draft', $4, $5, $6, $7, true, 0) \
         RETURNING {PIPELINE_COLUMNS}"
    );
    let row: PipelineRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(owner)
        .bind(&source)
        .bind(&target)
        .bind(&input.schedule)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Update an authored pipeline's status (`pausePipeline`/`resumePipeline`
/// for a pipeline that has no backing Dagster job — see
/// `routes::pipelines::pause`/`resume`).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn set_status(
    pool: &PgPool,
    id: &str,
    status: &str,
) -> Result<Option<Pipeline>, StoreError> {
    let sql = format!(
        "UPDATE pipeline_definition SET status = $2 WHERE id = $1 RETURNING {PIPELINE_COLUMNS}"
    );
    let row: Option<PipelineRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(status)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(Pipeline::from))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let pipeline = Pipeline {
            id: "pl-x".to_owned(),
            name: "n".to_owned(),
            kind: "batch".to_owned(),
            status: "draft".to_owned(),
            owner: "o".to_owned(),
            source: "s".to_owned(),
            target: "t".to_owned(),
            connector_id: None,
            source_asset_id: None,
            target_asset_id: None,
            schedule: "manual".to_owned(),
            last_run_at: "2026-01-01T00:00:00.000Z".to_owned(),
            next_run_at: None,
            sla_ok: true,
            freshness_lag_seconds: 0,
        };
        let value = serde_json::to_value(&pipeline).unwrap();
        for key in [
            "id",
            "name",
            "kind",
            "status",
            "owner",
            "source",
            "target",
            "schedule",
            "lastRunAt",
            "slaOk",
            "freshnessLagSeconds",
        ] {
            assert!(value.get(key).is_some(), "Pipeline is missing `{key}`");
        }
        assert!(value.get("connectorId").is_none());
        assert!(value.get("nextRunAt").is_none());
    }

    #[test]
    fn slug_id_lowercases_and_strips_punctuation() {
        let id = slug_id("Orders Hourly Rollup!!");
        assert!(id.starts_with("pl-orders-hourly-rollup-"));
    }

    #[test]
    fn slug_id_falls_back_to_new_when_name_has_no_alnum() {
        let id = slug_id("!!!");
        assert!(id.starts_with("pl-new-"));
    }

    #[test]
    fn radix36_matches_js_to_string_36() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
        assert_eq!(radix36(1_787_803_210_075), "mtazvdjv");
    }
}
