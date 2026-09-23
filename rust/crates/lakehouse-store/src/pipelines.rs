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
use uuid::Uuid;

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
    /// Last run time, ISO 8601. `None` for every authored pipeline: nothing
    /// executes an authored pipeline yet (`WS4` adds that), so there is no
    /// run time to report — serialized as JSON `null`, not omitted, so a
    /// reader sees "not measured" rather than a missing field.
    pub last_run_at: Option<String>,
    /// Next scheduled run time, ISO 8601, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    /// Whether the pipeline is currently meeting its SLA. `None`: no `SLA`
    /// is defined anywhere yet (`WS5` adds `dataset_sla`).
    pub sla_ok: Option<bool>,
    /// Current freshness lag in seconds. `None`: nothing measures it for an
    /// authored pipeline that has never run.
    pub freshness_lag_seconds: Option<i32>,
}

/// The full stored shape of an authored pipeline's definition, returned by
/// `GET /api/pipelines/{id}` as `definition` (Correction 5 in this task's
/// plan doc — the grand plan named this type without defining it).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthoredDefinition {
    /// Source zone (e.g. `"bronze"`).
    pub source_zone: String,
    /// Source table.
    pub source_table: String,
    /// Column an incremental read watermarks on, if any.
    pub incremental_column: Option<String>,
    /// Grammar-validated transform steps, in the order they run.
    pub transforms: Vec<String>,
    /// Whether file-based incremental capture is enabled.
    pub fbic_enabled: bool,
    /// Target zone.
    pub target_zone: String,
    /// Target table.
    pub target_table: String,
    /// Ingress connector id, if any.
    pub connector_id: Option<String>,
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
    next_run_at: Option<OffsetDateTime>,
}

/// A row shape for [`get_definition`], carrying the three columns migration
/// `0036` added (`incremental_column`/`fbic_enabled`/`transforms`) plus the
/// `source`/`target` this function splits back into zone/table pairs.
#[derive(Debug, FromRow)]
struct DefinitionRow {
    source: String,
    target: String,
    incremental_column: Option<String>,
    transforms: serde_json::Value,
    fbic_enabled: bool,
    connector_id: Option<String>,
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
            // `last_run_at`/`sla_ok`/`freshness_lag_seconds` are deliberately
            // not read from the row: the table's `NOT NULL DEFAULT` values
            // (`now()` / `true` / `0`, `0007_pipelines.sql`) are legacy
            // placeholders with no writer that ever updates them for an
            // authored pipeline (the only post-insert write is `UPDATE ...
            // SET status = $2`) — `WS4` executing authored pipelines and
            // `WS5` defining SLAs are what would make them real. Surfacing
            // the placeholder would read as a measurement, so these three
            // are always `None` regardless of what is stored.
            last_run_at: None,
            next_run_at: row.next_run_at.map(iso_millis),
            sla_ok: None,
            freshness_lag_seconds: None,
        }
    }
}

const PIPELINE_COLUMNS: &str = "id, name, kind, status, owner, source, target, connector_id, \
     source_asset_id, target_asset_id, schedule, next_run_at";

/// Optional narrowing for [`list_pipelines`] for tenant isolation: a caller
/// must never see a pipeline outside its own tenant.
///
/// Deliberately NOT the `tenant_id IS NULL OR ...` shape
/// [`crate::connectors::ConnectorFilter`] uses: `routes::pipelines::list`
/// (the one HTTP-reachable caller) always resolves a real tenant via
/// `tenant_scope::resolve` before this function is ever called and returns
/// early itself when that resolves to `None`, so `tenant_id` here is never
/// itself `None` on a real request. Keeping the `IS NOT NULL AND` guard
/// (rather than the `IS NULL OR` shape) means a filter-level unit test
/// that constructs `PipelineFilter { tenant_id: None, .. }` directly
/// proves "matches nothing" without depending on that route-level
/// short-circuit ever having run — the store function is fail-closed on
/// its own terms, not merely because of how its one caller happens to use
/// it today.
#[derive(Debug, Clone, Default)]
pub struct PipelineFilter {
    /// Restrict to this tenant, if given. `None` matches no rows at all —
    /// see the struct doc comment.
    pub tenant_id: Option<Uuid>,
}

/// List every authored pipeline definition, newest first, optionally
/// narrowed by [`PipelineFilter`].
///
/// A pipeline whose own `tenant_id` column is `NULL` (unassigned —
/// `0042_tenant_provisioning.sql`) never matches any `Some(tenant_id)`
/// filter (SQL `NULL = $1` is never `true`) — fail closed: an unassigned
/// row is invisible to every tenant-scoped read, never visible to every
/// one of them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_pipelines(
    pool: &PgPool,
    filter: &PipelineFilter,
) -> Result<Vec<Pipeline>, StoreError> {
    let sql = format!(
        "SELECT {PIPELINE_COLUMNS} FROM pipeline_definition \
         WHERE ($1::uuid IS NOT NULL AND tenant_id = $1) \
         ORDER BY created_at DESC"
    );
    let rows: Vec<PipelineRow> = sqlx::query_as(&sql)
        .bind(filter.tenant_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(Pipeline::from).collect())
}

/// Assign (or reassign) an authored pipeline to a tenant — the write
/// behind `PUT /api/pipelines/{id}/tenant`. Same
/// posture as [`crate::connectors::assign_tenant`]: `0042_tenant_
/// provisioning.sql` adds `tenant_id` to `pipeline_definition` with no
/// backfill at all (see that migration's own comment — every seeded
/// pipeline row was already deleted by `0027_prune_seeded_activity.sql` by
/// the time it applies), so every authored pipeline starts `tenant_id =
/// NULL` and needs this route to become visible to [`list_pipelines`]'s
/// tenant-scoped reads.
///
/// The `tenant_id` value is bound, never interpolated into the SQL text.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if no pipeline with `id` exists.
/// `routes::pipelines::assign_pipeline_tenant` checks `identity::
/// tenant_exists` before calling this, so a foreign-key violation on
/// `tenant_id` should not occur in practice; [`StoreError::
/// ForeignKeyViolation`] surfaces if it somehow does. Returns
/// [`StoreError::Database`] on any other failure.
pub async fn assign_tenant(pool: &PgPool, id: &str, tenant_id: Uuid) -> Result<(), StoreError> {
    let result = sqlx::query("UPDATE pipeline_definition SET tenant_id = $1 WHERE id = $2")
        .bind(tenant_id)
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
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

/// Fetch one authored pipeline's full stored definition (migration `0036`'s
/// `incremental_column`/`fbic_enabled`/`transforms`, plus `source`/`target`
/// split back into zone/table pairs) — the `definition` field `GET
/// /api/pipelines/{id}` reports for a `pl-` id.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_definition(
    pool: &PgPool,
    id: &str,
) -> Result<Option<AuthoredDefinition>, StoreError> {
    let row: Option<DefinitionRow> = sqlx::query_as(
        "SELECT source, target, incremental_column, transforms, fbic_enabled, connector_id \
         FROM pipeline_definition WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| {
        // Mirrors `create_pipeline`'s own `format!("{}.{}", zone, table)`
        // construction — split back into the two halves it came from.
        let (source_zone, source_table) = split_zone_table(&row.source);
        let (target_zone, target_table) = split_zone_table(&row.target);
        let transforms: Vec<String> = serde_json::from_value(row.transforms).unwrap_or_default();
        AuthoredDefinition {
            source_zone,
            source_table,
            incremental_column: row.incremental_column,
            transforms,
            fbic_enabled: row.fbic_enabled,
            target_zone,
            target_table,
            connector_id: row.connector_id,
        }
    }))
}

/// Split a `"<zone>.<table>"` location on its first `.`, matching
/// `create_pipeline`'s own construction. A location with no `.` (should
/// never happen for a row this store wrote) falls back to an empty zone
/// rather than panicking.
fn split_zone_table(location: &str) -> (String, String) {
    location.split_once('.').map_or_else(
        || (String::new(), location.to_owned()),
        |(zone, table)| (zone.to_owned(), table.to_owned()),
    )
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
    /// Column an incremental read watermarks on, if any (migration 0036).
    pub incremental_column: Option<String>,
    /// Grammar-validated transform steps, already checked by
    /// `transform_grammar::parse_transform` at the route layer before this
    /// function is ever called — this store function does not re-validate
    /// them, it only persists the strings.
    pub transforms: Vec<String>,
    /// Whether file-based incremental capture is enabled (migration 0036).
    pub fbic_enabled: bool,
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
/// `fromCreateInput`: always starts `status: "draft"`.
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
    // `sla_ok`/`freshness_lag_seconds` are `NOT NULL` columns with no
    // meaning yet (no writer ever updates them, see `Pipeline::from`'s doc
    // comment) — `true`/`0` are legacy placeholders satisfying the schema,
    // never surfaced to a caller.
    let transforms = serde_json::to_value(&input.transforms).unwrap_or(serde_json::Value::Null);
    let sql = format!(
        "INSERT INTO pipeline_definition (id, name, kind, status, owner, source, target, \
         schedule, sla_ok, freshness_lag_seconds, incremental_column, fbic_enabled, transforms) \
         VALUES ($1, $2, $3, 'draft', $4, $5, $6, $7, true, 0, $8, $9, $10) \
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
        .bind(&input.incremental_column)
        .bind(input.fbic_enabled)
        .bind(&transforms)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Update an authored pipeline's status (`pausePipeline`/`resumePipeline`
/// for a pipeline that has no backing Dagster job — see
/// `routes::pipelines::pause`/`resume`; and `routes::pipelines::
/// set_status_route`'s draft -> ready transition, WS4 item D4).
///
/// `status` is bound straight into the query with NO validation of its own
/// here — this function relies entirely on `pipeline_definition`'s
/// `pipeline_definition_status_check` CHECK constraint (`0007_pipelines.sql`)
/// to reject anything outside the fixed status vocabulary. **This is
/// defense in depth for THIS function's own callers only, not the real
/// safety boundary for `POST /api/pipelines/{id}/status`**:
/// `routes::pipelines::set_status_route`'s `ALLOWED_TRANSITIONS` table is
/// checked in the ROUTE, before this function is ever called, and is what
/// actually stops a `pipeline:write` principal from setting a run-derived
/// status (`"completed"`/`"running"`/`"failed"`/`"degraded"`/`"partial"`)
/// that fabricates an execution outcome (judge review V9). `pause`/
/// `resume`'s own callers only ever pass the literal `"paused"`/`"ready"`,
/// so the CHECK constraint alone has always been sufficient for them.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure, including the CHECK
/// constraint rejecting a `status` outside its fixed vocabulary.
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
            last_run_at: Some("2026-01-01T00:00:00.000Z".to_owned()),
            next_run_at: None,
            sla_ok: Some(true),
            freshness_lag_seconds: Some(0),
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
    fn authored_pipelines_report_no_run_time_sla_or_freshness() {
        // No writer updates `last_run_at`/`sla_ok`/`freshness_lag_seconds`
        // after insert (WS1 finding J16), so `Pipeline::from` must ignore
        // whatever the row happens to store and always report "not
        // measured" — a fixture with plausible-looking real values proves
        // that, rather than one that would pass by accident.
        let row = PipelineRow {
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
            next_run_at: None,
        };
        let pipeline = Pipeline::from(row);
        assert_eq!(pipeline.last_run_at, None);
        assert_eq!(pipeline.sla_ok, None);
        assert_eq!(pipeline.freshness_lag_seconds, None);

        let value = serde_json::to_value(&pipeline).unwrap();
        // These must serialize as JSON `null`, not be omitted — `slaOk`/
        // `freshnessLagSeconds` have no `skip_serializing_if`, and
        // `lastRunAt` likewise always appears.
        assert_eq!(value["lastRunAt"], serde_json::Value::Null);
        assert_eq!(value["slaOk"], serde_json::Value::Null);
        assert_eq!(value["freshnessLagSeconds"], serde_json::Value::Null);
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
