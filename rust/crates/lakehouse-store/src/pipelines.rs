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

use serde::{Deserialize, Serialize};
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
    /// Last run time, ISO 8601; `None` when it has never run. Read from the
    /// stored column: `0048_pipeline_never_run.sql` dropped the `now()`
    /// default that made every freshly authored pipeline lie about having
    /// just run, so a stored `NULL` here is now an honest "never ran", not
    /// a placeholder to override.
    pub last_run_at: Option<String>,
    /// Next scheduled run time, ISO 8601, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_run_at: Option<String>,
    /// Whether the pipeline is currently meeting its SLA. `None`: no `SLA`
    /// is defined anywhere yet (`WS5` adds `dataset_sla`) — unlike
    /// `last_run_at`/`freshness_lag_seconds`, no migration has made this
    /// column's `NOT NULL DEFAULT true` mean anything real, so it stays a
    /// hardcoded `None` regardless of what is stored (would otherwise
    /// report every pipeline "on SLA").
    pub sla_ok: Option<bool>,
    /// Current freshness lag in seconds, read from the stored column
    /// (`0048_pipeline_never_run.sql` made it honestly nullable): `None`
    /// until something measures it, never a fabricated `0`.
    pub freshness_lag_seconds: Option<i32>,
    /// Per-pipeline retry cap (Plan 1c / migration 0051): the count
    /// `authored_factory._op_for_pipeline` passes as
    /// `RetryPolicy.max_retries` when it builds the op. `0..=5`,
    /// default `2`; the column's CHECK constraint is the database-level
    /// safety net, the route layer's 400 is the user-facing one.
    pub max_retries: i16,
    /// What this pipeline is for, as its author described it
    /// (`0047_pipeline_description.sql`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The pipeline ids (authored `pl-…` or Dagster job names) whose
    /// successful runs must precede this one's (migration `0052`); empty
    /// for a pipeline that runs only on its own trigger. The factory's
    /// `authored__<id>_after` sensor is built only when this is non-empty.
    pub depends_on: Vec<String>,
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
    /// Per-pipeline retry cap (`Pipeline::max_retries`, migration 0051):
    /// passed to `authored_factory._op_for_pipeline` so the built op's
    /// `RetryPolicy.max_retries` overrides the count of
    /// `op_metadata.DEFAULT_RETRY_POLICY` while keeping its delay,
    /// backoff and jitter.
    pub max_retries: i16,
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
    last_run_at: Option<OffsetDateTime>,
    next_run_at: Option<OffsetDateTime>,
    freshness_lag_seconds: Option<i32>,
    max_retries: i16,
    description: Option<String>,
    depends_on: Vec<String>,
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
    max_retries: i16,
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
            // `last_run_at`/`freshness_lag_seconds` are read for real:
            // `0048_pipeline_never_run.sql` dropped their `NOT NULL
            // DEFAULT` (`now()` / `0`, `0007_pipelines.sql`), so a `NULL`
            // here is now an honest "never ran" / "never measured", not a
            // fabricated placeholder to hide. `sla_ok` is NOT read — no
            // migration has touched its `NOT NULL DEFAULT true`, so every
            // row would report "on SLA" — it stays a hardcoded `None`
            // until `WS5`'s `dataset_sla` gives it real meaning.
            last_run_at: row.last_run_at.map(iso_millis),
            next_run_at: row.next_run_at.map(iso_millis),
            sla_ok: None,
            freshness_lag_seconds: row.freshness_lag_seconds,
            max_retries: row.max_retries,
            description: row.description,
            depends_on: row.depends_on,
        }
    }
}

const PIPELINE_COLUMNS: &str = "id, name, kind, status, owner, source, target, connector_id, \
     source_asset_id, target_asset_id, schedule, last_run_at, next_run_at, \
freshness_lag_seconds, max_retries, description, depends_on";

/// `RETURNING` clause for the transactional `create_pipeline` /
/// `update_pipeline` / `delete_pipeline`-read paths. Wider than
/// [`PIPELINE_COLUMNS`] so the snapshot (`incremental_column` /
/// `transforms` / `fbic_enabled`) can be built from the same row that
/// the `Pipeline` result is built from, keeping the two shapes from
/// drifting.
const FULL_PIPELINE_COLUMNS: &str = "id, name, kind, status, owner, source, target, connector_id, \
     source_asset_id, target_asset_id, schedule, last_run_at, next_run_at, \
freshness_lag_seconds, max_retries, description, depends_on, incremental_column, \
     transforms, fbic_enabled";

/// The full editable state of a pipeline at one point in time. Persisted
/// to `pipeline_definition_version.snapshot` (migration `0053`) inside
/// the same transaction as the matching `created`/`updated`/`restored`/
/// `deleted` row write, so a `created`/`updated`/`restored` snapshot is
/// the POST-write row and a `deleted` snapshot is the pre-delete row.
///
/// Mirrors the editable shape `UpdatePipelineInput` carries — minus
/// `name` and `status`, which the input does not own — plus `name` and
/// `status` from the post-write row. A future restore route rebuilds an
/// `UpdatePipelineInput` from this shape (name and status are not
/// editable, so they are not restored; the live row's name and status
/// survive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineDefinitionSnapshot {
    /// `pipeline_definition.kind`.
    pub kind: String,
    /// `pipeline_definition.source` split on the first `.`.
    pub source_zone: String,
    /// `pipeline_definition.source` split on the first `.`.
    pub source_table: String,
    /// `pipeline_definition.incremental_column` (migration `0036`).
    pub incremental_column: Option<String>,
    /// `pipeline_definition.transforms` decoded from `jsonb`.
    pub transforms: Vec<String>,
    /// `pipeline_definition.fbic_enabled` (migration `0036`).
    pub fbic_enabled: bool,
    /// `pipeline_definition.target` split on the first `.`.
    pub target_zone: String,
    /// `pipeline_definition.target` split on the first `.`.
    pub target_table: String,
    /// `pipeline_definition.schedule`.
    pub schedule: String,
    /// `pipeline_definition.owner`.
    pub owner: String,
    /// `pipeline_definition.description` (migration `0047`).
    pub description: Option<String>,
    /// `pipeline_definition.max_retries` (migration `0051`).
    pub max_retries: i16,
    /// `pipeline_definition.depends_on` (migration `0052`).
    pub depends_on: Vec<String>,
    /// `pipeline_definition.name`. Read from the post-write row; not
    /// part of `UpdatePipelineInput`, so the restore route does not
    /// write it back.
    pub name: String,
    /// `pipeline_definition.status`. Same posture as `name`.
    pub status: String,
}

/// Metadata-only entry of `pipeline_definition_version`, returned by
/// `list_definition_versions` and the list half of the version-history
/// routes. Does NOT carry `snapshot` — the list payload is small enough
/// to render without per-row payloads, and the route's two-list shape
/// (newest-first metadata, then a per-version snapshot get) is the
/// diff-friendly contract a future UI reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineVersionMeta {
    /// `pipeline_definition_version.version` (1-based, gap-free per
    /// `pipeline_id`).
    pub version: i32,
    /// `"created" | "updated" | "restored" | "deleted"` — fixed
    /// vocabulary from migration `0053`'s CHECK constraint.
    pub event: String,
    /// `pipeline_definition_version.changed_by` — `principal.id.uuid()`
    /// from the route, `None` when no principal was present.
    pub changed_by: Option<Uuid>,
    /// `pipeline_definition_version.changed_at`, ISO 8601 milliseconds.
    pub changed_at: String,
}

#[derive(Debug, FromRow)]
struct VersionRow {
    version: i32,
    event: String,
    changed_by: Option<Uuid>,
    changed_at: OffsetDateTime,
}

impl From<VersionRow> for PipelineVersionMeta {
    fn from(row: VersionRow) -> Self {
        Self {
            version: row.version,
            event: row.event,
            changed_by: row.changed_by,
            changed_at: iso_millis(row.changed_at),
        }
    }
}

/// Insert a single `pipeline_definition_version` row inside an existing
/// transaction. Every `created`/`updated`/`restored`/`deleted` event is
/// written this way — the same transaction holds the matching live
/// write, so a snapshot can never appear without its live write (or
/// vice versa). `version` is computed inside this call from the
/// `MAX(version)` for `pipeline_id`, so a `created` row is always 1 and
/// every subsequent event is one greater than the last.
///
/// `event` is one of `"created"`, `"updated"`, `"restored"`,
/// `"deleted"`; the migration's CHECK constraint is the database-level
/// guard, this caller's contract. `changed_by` is bound verbatim and
/// may be `None`.
async fn insert_definition_version(
    tx: &mut sqlx::PgConnection,
    pipeline_id: &str,
    snapshot: &PipelineDefinitionSnapshot,
    event: &str,
    changed_by: Option<Uuid>,
) -> Result<(), StoreError> {
    let snapshot_value = serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null);
    let row: (i32,) = sqlx::query_as(
        "INSERT INTO pipeline_definition_version \
            (pipeline_id, version, snapshot, event, changed_by) \
         SELECT $1, COALESCE(MAX(version), 0) + 1, $2, $3, $4 \
           FROM pipeline_definition_version WHERE pipeline_id = $1 \
         RETURNING version",
    )
    .bind(pipeline_id)
    .bind(snapshot_value)
    .bind(event)
    .bind(changed_by)
    .fetch_one(tx)
    .await?;
    let _ = row; // inserted; the version row's auto-incrementing gap-free
    // sequence is the only thing the caller would care about, and a
    // successful INSERT is the proof. `row.0` is left unused here
    // because every caller obtains `version` from a separate list/get
    // call rather than threading the value back through the store
    // function signatures (the snapshot's identity is its `version`
    // column, not the caller's local knowledge).
    Ok(())
}

/// Every column the snapshot needs, plus the columns [`Pipeline`] needs
/// to construct its return value. `RETURNING` clauses that build both
/// the live row and the version snapshot use this row, never
/// [`PipelineRow`], so the two shapes cannot drift.
#[derive(Debug, FromRow)]
struct FullPipelineRow {
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
    last_run_at: Option<OffsetDateTime>,
    next_run_at: Option<OffsetDateTime>,
    freshness_lag_seconds: Option<i32>,
    max_retries: i16,
    description: Option<String>,
    depends_on: Vec<String>,
    incremental_column: Option<String>,
    transforms: serde_json::Value,
    fbic_enabled: bool,
}

impl From<FullPipelineRow> for Pipeline {
    fn from(row: FullPipelineRow) -> Self {
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
            last_run_at: row.last_run_at.map(iso_millis),
            next_run_at: row.next_run_at.map(iso_millis),
            sla_ok: None,
            freshness_lag_seconds: row.freshness_lag_seconds,
            max_retries: row.max_retries,
            description: row.description,
            depends_on: row.depends_on,
        }
    }
}

impl From<&FullPipelineRow> for PipelineDefinitionSnapshot {
    fn from(row: &FullPipelineRow) -> Self {
        let (source_zone, source_table) = split_zone_table(&row.source);
        let (target_zone, target_table) = split_zone_table(&row.target);
        let transforms: Vec<String> =
            serde_json::from_value(row.transforms.clone()).unwrap_or_default();
        Self {
            kind: row.kind.clone(),
            source_zone,
            source_table,
            incremental_column: row.incremental_column.clone(),
            transforms,
            fbic_enabled: row.fbic_enabled,
            target_zone,
            target_table,
            schedule: row.schedule.clone(),
            owner: row.owner.clone(),
            description: row.description.clone(),
            max_retries: row.max_retries,
            depends_on: row.depends_on.clone(),
            name: row.name.clone(),
            status: row.status.clone(),
        }
    }
}

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
    /// Every row, whatever its tenant, unassigned rows included. Only for a
    /// Platform Admin (`*:*`) with no tenant of their own, who otherwise
    /// saw no authored pipeline at all; the route decides, this function
    /// only obeys. `false` keeps the fail-closed tenant rule above.
    pub all_tenants: bool,
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
         WHERE ($2 OR ($1::uuid IS NOT NULL AND tenant_id = $1)) \
         ORDER BY created_at DESC"
    );
    let rows: Vec<PipelineRow> = sqlx::query_as(&sql)
        .bind(filter.tenant_id)
        .bind(filter.all_tenants)
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
        "SELECT source, target, incremental_column, transforms, fbic_enabled, connector_id, \
         max_retries FROM pipeline_definition WHERE id = $1",
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
            max_retries: row.max_retries,
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
    /// What the pipeline is for, in the author's words (migration 0047).
    pub description: Option<String>,
    /// Per-pipeline retry cap (Plan 1c / migration 0051): passed to
    /// `authored_factory._op_for_pipeline` as `RetryPolicy.max_retries`
    /// when it builds the op. The route layer rejects out-of-range
    /// values with 400, so this field is `0..=5`. `None` here resolves to
    /// the migration's `DEFAULT 2` on insert (the SQL omits the column,
    /// leaving the schema default in place), so a caller that does not
    /// mention `max_retries` gets the same value as a missing column.
    pub max_retries: Option<i16>,
    /// The tenant the pipeline belongs to: the creator's active tenant.
    /// `None` leaves the row unassigned (`0042_tenant_provisioning.sql`),
    /// which every tenant-scoped read then skips; before this field every
    /// console-created pipeline landed there and never appeared on the
    /// Pipelines list, the page the create form returns to.
    pub tenant_id: Option<Uuid>,
    /// Upstream pipeline ids whose SUCCESS runs must precede this one's.
    /// Defaults to empty when the field is absent (route omits it, the
    /// migration's `DEFAULT '{}'` fills the column). Migration `0052`.
    pub depends_on: Vec<String>,
}

const DEFAULT_OWNER: &str = "Current user";

/// Create an authored pipeline, matching `mock/pipelines.ts`'s
/// `fromCreateInput`: always starts `status: "draft"`.
///
/// Writes exactly one `pipeline_definition_version` row inside the same
/// transaction (Plan R4 2b). `changed_by` is the principal that requested
/// the create; `routes::pipelines::generate` passes the principal it has
/// (matching the audit-side pattern) so even non-`POST /api/pipelines`
/// creates get a `created` version row.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken, or
/// [`StoreError::Database`] on any other failure.
pub async fn create_pipeline(
    pool: &PgPool,
    input: &CreatePipelineInput,
    changed_by: Option<Uuid>,
) -> Result<Pipeline, StoreError> {
    let id = slug_id(&input.name);
    let source = format!("{}.{}", input.source_zone, input.source_table);
    let target = format!("{}.{}", input.target_zone, input.target_table);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    // `sla_ok`, `last_run_at`, `freshness_lag_seconds` are deliberately
    // absent from this INSERT's column list: `sla_ok` keeps its schema
    // default (`NOT NULL DEFAULT true`, still a placeholder — see
    // `Pipeline::from`'s doc comment); `last_run_at`/`freshness_lag_seconds`
    // lost their defaults in `0048_pipeline_never_run.sql`, so omitting
    // them here inserts `NULL` — a freshly authored pipeline has honestly
    // never run, not a fabricated "just now".
    let transforms = serde_json::to_value(&input.transforms).unwrap_or(serde_json::Value::Null);
    // `max_retries` (migration 0051): a present input is bound as-is; an
    // absent input resolves to the migration's `DEFAULT 2` here so the
    // row carries the same value either way (the column has no other
    // source — there is no per-row "explicitly defaulted" distinction
    // worth preserving).
    let max_retries = input.max_retries.unwrap_or(2);
    let sql = format!(
        "INSERT INTO pipeline_definition (id, name, kind, status, owner, source, target, \
schedule, description, incremental_column, fbic_enabled, transforms, \
         max_retries, tenant_id, depends_on) \
         VALUES ($1, $2, $3, 'draft', $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) \
         RETURNING {FULL_PIPELINE_COLUMNS}"
    );
    let mut tx = pool.begin().await?;
    let row: FullPipelineRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(owner)
        .bind(&source)
        .bind(&target)
        .bind(&input.schedule)
        .bind(&input.description)
        .bind(&input.incremental_column)
        .bind(input.fbic_enabled)
        .bind(&transforms)
        .bind(max_retries)
        .bind(input.tenant_id)
        .bind(&input.depends_on)
        .fetch_one(&mut *tx)
        .await?;
    let snapshot = PipelineDefinitionSnapshot::from(&row);
    insert_definition_version(&mut tx, &row.id, &snapshot, "created", changed_by).await?;
    tx.commit().await?;
    Ok(Pipeline::from(row))
}

/// The editable half of an authored pipeline, for [`update_pipeline`].
/// `name` is not here: the id is derived from it at creation
/// ([`slug_id`]), and the Dagster job name is derived from the id, so a
/// rename would leave the id describing a name the pipeline no longer has.
#[derive(Debug, Clone)]
pub struct UpdatePipelineInput {
    /// Pipeline kind.
    pub kind: String,
    /// Source zone.
    pub source_zone: String,
    /// Source table.
    pub source_table: String,
    /// Incremental watermark column, if any.
    pub incremental_column: Option<String>,
    /// Grammar-validated transform steps; validated at the route layer,
    /// exactly as for [`create_pipeline`].
    pub transforms: Vec<String>,
    /// Whether file-based incremental capture is enabled.
    pub fbic_enabled: bool,
    /// Target zone.
    pub target_zone: String,
    /// Target table.
    pub target_table: String,
    /// Schedule label.
    pub schedule: String,
    /// Owner; unchanged when absent.
    pub owner: Option<String>,
    /// Description; cleared when absent.
    pub description: Option<String>,
    /// Per-pipeline retry cap (`Pipeline::max_retries`, migration 0051).
    /// `None` leaves the stored value alone — exactly the same shape as
    /// [`Self::owner`], and the route layer validates `0..=5` BEFORE
    /// this function is called so this code never sees an out-of-range
    /// value (the CHECK constraint in `0051_pipeline_max_retries.sql` is
    /// defense in depth, not the primary safety boundary).
    pub max_retries: Option<i16>,
    /// Upstream pipeline ids whose SUCCESS runs must precede this one's
    /// (migration `0052`). Defaults to empty (the route passes `[]` when
    /// the body omits the field, and the migration's `DEFAULT '{}'`
    /// accepts the empty value).
    pub depends_on: Vec<String>,
}

/// Replace an authored pipeline's definition. Its status is left as it
/// is: editing a `ready` pipeline keeps it runnable with the new
/// definition, which the orchestrator picks up on its next reload.
///
/// Writes exactly one `pipeline_definition_version` row inside the same
/// transaction (Plan R4 2b) with `event = "updated"`. The matching
/// restore path goes through [`restore_pipeline`] with `event =
/// "restored"`, so the governance trail distinguishes "edited" from
/// "replayed a stored snapshot" — the design promise of plan R4 2b
/// bullet 3.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure. `Ok(None)` when no
/// pipeline with `id` exists.
pub async fn update_pipeline(
    pool: &PgPool,
    id: &str,
    input: &UpdatePipelineInput,
    changed_by: Option<Uuid>,
) -> Result<Option<Pipeline>, StoreError> {
    update_pipeline_with_event(pool, id, input, changed_by, "updated").await
}

/// Restore an authored pipeline's definition to a previously-snapshotted
/// state. Same-transaction write as [`update_pipeline`], but records the
/// version row's `event` as `"restored"` so the governance trail
/// distinguishes "replayed a stored snapshot" from a plain edit (Plan
/// R4 2b design bullet 3). The caller rebuilds the
/// [`UpdatePipelineInput`] from a [`PipelineDefinitionSnapshot`] returned
/// by `pipelines::get_definition_version`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure. `Ok(None)` when no
/// pipeline with `id` exists.
pub async fn restore_pipeline(
    pool: &PgPool,
    id: &str,
    input: &UpdatePipelineInput,
    changed_by: Option<Uuid>,
) -> Result<Option<Pipeline>, StoreError> {
    update_pipeline_with_event(pool, id, input, changed_by, "restored").await
}

/// Shared implementation behind [`update_pipeline`] (event `"updated"`)
/// and [`restore_pipeline`] (event `"restored"`). `event` is bound
/// verbatim into the new `pipeline_definition_version` row's `event`
/// column, so the migration's CHECK constraint is the only thing that
/// guards the `"created" | "updated" | "restored" | "deleted"`
/// vocabulary — there is no Rust-side enum here on purpose, because the
/// vocabulary is the database's contract, not the store's.
async fn update_pipeline_with_event(
    pool: &PgPool,
    id: &str,
    input: &UpdatePipelineInput,
    changed_by: Option<Uuid>,
    event: &str,
) -> Result<Option<Pipeline>, StoreError> {
    let source = format!("{}.{}", input.source_zone, input.source_table);
    let target = format!("{}.{}", input.target_zone, input.target_table);
    let transforms = serde_json::to_value(&input.transforms).unwrap_or(serde_json::Value::Null);
    // `max_retries` (migration 0051): `COALESCE($N, max_retries)` —
    // the route layer rejects out-of-range values with 400 first, so
    // the CHECK constraint here is defense in depth for whatever
    // happens to bind straight into the column.
    let sql = format!(
        "UPDATE pipeline_definition SET kind = $2, source = $3, target = $4, schedule = $5, \
         description = $6, incremental_column = $7, fbic_enabled = $8, transforms = $9, \
owner = COALESCE($10, owner), max_retries = COALESCE($11, max_retries), \
         depends_on = $12 \
         WHERE id = $1 RETURNING {FULL_PIPELINE_COLUMNS}"
    );
    let mut tx = pool.begin().await?;
    let row: Option<FullPipelineRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(&input.kind)
        .bind(&source)
        .bind(&target)
        .bind(&input.schedule)
        .bind(&input.description)
        .bind(&input.incremental_column)
        .bind(input.fbic_enabled)
        .bind(&transforms)
        .bind(&input.owner)
        .bind(input.max_retries)
        .bind(&input.depends_on)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else {
        // The UPDATE matched zero rows: rollback the transaction (a no-op
        // on a read-only tx, but kept for symmetry with the success path)
        // and return None so the caller surfaces 404.
        tx.rollback().await?;
        return Ok(None);
    };
    let snapshot = PipelineDefinitionSnapshot::from(&row);
    insert_definition_version(&mut tx, &row.id, &snapshot, event, changed_by).await?;
    tx.commit().await?;
    Ok(Some(Pipeline::from(row)))
}

/// Delete an authored pipeline. `true` when a row was deleted, `false`
/// when no pipeline with `id` existed.
///
/// Writes a final `pipeline_definition_version` row with `event =
/// "deleted"` inside the same transaction as the `DELETE` (Plan R4 2b).
/// The snapshot captures the PRE-delete state, so a caller reading the
/// `deleted` row after the fact sees what the pipeline was, not what the
/// table now says (the table says "no such row"). A `false` return — no
/// pipeline with `id` existed — writes no version row (no snapshot to
/// capture).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn delete_pipeline(
    pool: &PgPool,
    id: &str,
    changed_by: Option<Uuid>,
) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let row: Option<FullPipelineRow> = sqlx::query_as(&format!(
        "SELECT {FULL_PIPELINE_COLUMNS} FROM pipeline_definition WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.rollback().await?;
        return Ok(false);
    };
    let snapshot = PipelineDefinitionSnapshot::from(&row);
    let done = sqlx::query("DELETE FROM pipeline_definition WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if done.rows_affected() == 0 {
        // The row vanished between the SELECT and the DELETE (a
        // concurrent writer). The transaction rolls back on `Drop`, so
        // the just-captured snapshot is discarded — and there is no
        // pre-delete state to record anyway.
        tx.rollback().await?;
        return Ok(false);
    }
    insert_definition_version(&mut tx, id, &snapshot, "deleted", changed_by).await?;
    tx.commit().await?;
    Ok(true)
}

/// One authored pipeline the orchestrator should build a job for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunnablePipeline {
    /// The pipeline id (`pl-...`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// `ready` or `paused`: a paused pipeline still has a job (so it can be
    /// run by hand); only its schedule is stopped.
    pub status: String,
    /// Schedule label as authored: a five-field cron, or anything else for
    /// "on demand only".
    pub schedule: String,
    /// The stored definition the job is built from.
    pub definition: AuthoredDefinition,
    /// Upstream pipeline ids whose SUCCESS runs must precede this one's
    /// (migration `0052`). The factory's import-time `GET
    /// /api/pipelines/runnable` call must carry it, otherwise the
    /// `authored__<id>_after` sensor would have to make a second
    /// round-trip per pipeline to read it — see `authored_factory.py`'s
    /// docstring on "every way this degrades to empty lists without
    /// raising" for why a second fetch would defeat that.
    pub depends_on: Vec<String>,
}

/// Every authored pipeline in status `ready` or `paused`, across all
/// tenants: the orchestrator runs every tenant's pipelines, the way
/// `connectors::list_ingestible_connectors` feeds every tenant's ingest.
/// Only `GET /api/pipelines/runnable` reads this, and that route refuses
/// anyone but a service identity.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn list_runnable_pipelines(pool: &PgPool) -> Result<Vec<RunnablePipeline>, StoreError> {
    let rows: Vec<(String, String, String, String, Vec<String>)> = sqlx::query_as(
        "SELECT id, name, status, schedule, depends_on FROM pipeline_definition \
         WHERE status IN ('ready', 'paused') ORDER BY created_at",
    )
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, name, status, schedule, depends_on) in rows {
        // A row deleted between the two reads simply drops out.
        if let Some(definition) = get_definition(pool, &id).await? {
            out.push(RunnablePipeline {
                id,
                name,
                status,
                schedule,
                definition,
                depends_on,
            });
        }
    }
    Ok(out)
}

/// Recorded dedup of a single orchestrator-emitted pipeline-run event
/// (the `run_failure_sensor` in `dagster/dispar_orchestrate/pipeline_events.py`
/// posts each one to `POST /api/pipelines/events/run-failed`, which calls
/// this BEFORE evaluating alert rules). Keyed by `(run_id, kind)` so a
/// sensor retry never double-alerts: a second INSERT for the same row is
/// a no-op, and the bool returned is `true` only when a row was newly
/// inserted. The same table backs the kinds 1f adds (`slow`,
/// `volume_drop`, `late`) — `record_pipeline_run_event` is the only writer
/// for them too.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the INSERT itself fails.
pub async fn record_pipeline_run_event(
    pool: &PgPool,
    run_id: &str,
    pipeline_id: &str,
    kind: &str,
) -> Result<bool, StoreError> {
    let row: Option<(String,)> = sqlx::query_as(
        "INSERT INTO pipeline_run_event (run_id, pipeline_id, kind) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (run_id, kind) DO NOTHING \
         RETURNING run_id",
    )
    .bind(run_id)
    .bind(pipeline_id)
    .bind(kind)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Per-pipeline service-level agreement (plan 1f, migration
/// `0050_pipeline_sla.sql`).
///
/// The detail route, the runs route, and the alert pipeline read this to
/// decide whether a run was over its duration SLA, whether the pipeline is
/// late (no successful run inside `late_after_seconds`), and whether to
/// fire `pipeline_slow` / `pipeline_late` / `pipeline_volume_drop` alerts.
/// Mirrors the camelCase shape returned by `GET /api/pipelines/{id}/sla`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineSla {
    /// Pipeline identifier (Dagster job name or upstream `pl-` slug).
    pub pipeline_id: String,
    /// Per-run duration ceiling, in whole seconds. `None` = no duration
    /// SLA; the runs route emits `overDuration: null` for every run.
    pub max_duration_seconds: Option<i32>,
    /// Late threshold, in whole seconds since the last successful run.
    /// `None` = no late threshold; the detail route emits `late: null`.
    pub late_after_seconds: Option<i32>,
    /// Who last wrote this row.
    pub updated_by: Uuid,
    /// ISO 8601 (millisecond precision, UTC).
    pub updated_at: String,
}

/// Read the SLA for a single pipeline, or `None` if none has been set.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn get_pipeline_sla(
    pool: &PgPool,
    pipeline_id: &str,
) -> Result<Option<PipelineSla>, StoreError> {
    let row: Option<SlaRow> = sqlx::query_as(
        "SELECT pipeline_id, max_duration_seconds, late_after_seconds, \
                updated_by, updated_at \
           FROM pipeline_sla WHERE pipeline_id = $1",
    )
    .bind(pipeline_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(PipelineSla::from))
}

/// Read SLAs for many pipelines in one query (plan 1f). Used by the list
/// route to avoid N+1 round trips when the page holds 100+ jobs. Pipelines
/// without an SLA row are simply absent from the returned map.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_pipeline_slas(
    pool: &PgPool,
    pipeline_ids: &[&str],
) -> Result<std::collections::HashMap<String, PipelineSla>, StoreError> {
    if pipeline_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let rows: Vec<SlaRow> = sqlx::query_as(
        "SELECT pipeline_id, max_duration_seconds, late_after_seconds, \
                updated_by, updated_at \
           FROM pipeline_sla WHERE pipeline_id = ANY($1)",
    )
    .bind(pipeline_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let sla = PipelineSla::from(r);
            (sla.pipeline_id.clone(), sla)
        })
        .collect())
}

/// Upsert the SLA for a single pipeline.
///
/// `None` for either threshold clears that column (an empty PUT body is a
/// "no SLA at all" request; `Some(0)` is rejected by the CHECK constraint
/// in `0050_pipeline_sla.sql` and surfaces as [`StoreError::Database`]).
/// Idempotent: the PUT route calls this with the authenticated principal
/// so the audit trail records who set what.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure, including a
/// CHECK-constraint violation when the caller somehow bypassed the
/// route-level guard.
pub async fn upsert_pipeline_sla(
    pool: &PgPool,
    pipeline_id: &str,
    max_duration_seconds: Option<i32>,
    late_after_seconds: Option<i32>,
    updated_by: Uuid,
) -> Result<PipelineSla, StoreError> {
    let row: SlaRow = sqlx::query_as(
        "INSERT INTO pipeline_sla \
            (pipeline_id, max_duration_seconds, late_after_seconds, updated_by) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (pipeline_id) DO UPDATE SET \
            max_duration_seconds = EXCLUDED.max_duration_seconds, \
            late_after_seconds   = EXCLUDED.late_after_seconds, \
            updated_by           = EXCLUDED.updated_by, \
            updated_at           = now() \
         RETURNING pipeline_id, max_duration_seconds, late_after_seconds, \
                   updated_by, updated_at",
    )
    .bind(pipeline_id)
    .bind(max_duration_seconds)
    .bind(late_after_seconds)
    .bind(updated_by)
    .fetch_one(pool)
    .await?;
    Ok(PipelineSla::from(row))
}

#[derive(Debug, Clone, FromRow)]
struct SlaRow {
    pipeline_id: String,
    max_duration_seconds: Option<i32>,
    late_after_seconds: Option<i32>,
    updated_by: Uuid,
    updated_at: OffsetDateTime,
}

impl From<SlaRow> for PipelineSla {
    fn from(row: SlaRow) -> Self {
        Self {
            pipeline_id: row.pipeline_id,
            max_duration_seconds: row.max_duration_seconds,
            late_after_seconds: row.late_after_seconds,
            updated_by: row.updated_by,
            updated_at: iso_millis(row.updated_at),
        }
    }
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

/// List every version row for `pipeline_id`, newest first (largest
/// `version` first; the migration's gap-free allocator means the most
/// recent event is the one with the highest number). The list shape has
/// NO `snapshot` field — `GET /api/pipelines/{id}/versions` is the
/// metadata-only shape a UI lists, and the matching `GET
/// /api/pipelines/{id}/versions/{version}` is the per-row snapshot get.
///
/// An unknown `pipeline_id` returns an empty list, not an error: a
/// caller that has never seen the pipeline (e.g. a UI listing an id the
/// page just navigated to) gets the same shape whether the pipeline has
/// no history yet or has never existed.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_definition_versions(
    pool: &PgPool,
    pipeline_id: &str,
) -> Result<Vec<PipelineVersionMeta>, StoreError> {
    let rows: Vec<VersionRow> = sqlx::query_as(
        "SELECT version, event, changed_by, changed_at \
           FROM pipeline_definition_version \
          WHERE pipeline_id = $1 \
          ORDER BY version DESC",
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(PipelineVersionMeta::from).collect())
}

/// Read one version's snapshot (the editable state at that moment).
/// `Ok(None)` for an unknown `(pipeline_id, version)` pair — the
/// `restore` route's 404 path.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_definition_version(
    pool: &PgPool,
    pipeline_id: &str,
    version: i32,
) -> Result<Option<PipelineDefinitionSnapshot>, StoreError> {
    let row: Option<(serde_json::Value,)> = sqlx::query_as(
        "SELECT snapshot FROM pipeline_definition_version \
          WHERE pipeline_id = $1 AND version = $2",
    )
    .bind(pipeline_id)
    .bind(version)
    .fetch_optional(pool)
    .await?;
    let Some((value,)) = row else {
        return Ok(None);
    };
    let snapshot: PipelineDefinitionSnapshot = serde_json::from_value(value).map_err(|err| {
        // A snapshot we cannot decode into the typed shape is a database
        // corruption symptom — the migration's shape is the single source of
        // truth, and a row that diverges from it is not safe to return as
        // `Ok(Some(_))`. Wrap the serde error in a `sqlx::Error::Decode` so
        // the crate's existing `StoreError::Database` -> `ApiError::Internal`
        // mapping is the honest classifier: "the row exists but its payload
        // is unreadable" is a 500-class condition, not a caller error, and
        // the `Database` variant's `Display` is the fixed `"database error"`
        // — no upstream serde text (column/line/expected-type fragments) ever
        // reaches the wire (see `error.rs`'s no-leak rule). The pipeline_id
        // and version go into the [`tracing`] span at the route layer for
        // the operator who has to reconcile the row.
        StoreError::Database(sqlx::Error::Decode(Box::new(err)))
    })?;
    Ok(Some(snapshot))
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
            max_retries: 2,
            description: None,
            depends_on: Vec::new(),
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
            "dependsOn",
            "maxRetries",
        ] {
            assert!(value.get(key).is_some(), "Pipeline is missing `{key}`");
        }
        assert!(value.get("connectorId").is_none());
        assert!(value.get("nextRunAt").is_none());
    }

    #[test]
    fn authored_pipeline_with_no_stored_run_reports_never_ran_and_no_sla() {
        // `0048_pipeline_never_run.sql` dropped `last_run_at`/
        // `freshness_lag_seconds`'s fabricated `now()`/`0` defaults, so a
        // freshly authored row genuinely stores `NULL` for both — this
        // fixture proves `Pipeline::from` reports that faithfully rather
        // than substituting a placeholder. `sla_ok` still has no migration
        // behind it (`WS5`), so it stays hardcoded `None` regardless of
        // what the row would say.
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
            last_run_at: None,
            next_run_at: None,
            freshness_lag_seconds: None,
            max_retries: 2,
            description: None,
            depends_on: Vec::new(),
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
    fn authored_pipeline_with_a_stored_run_reports_it_but_still_no_sla() {
        // The mirror of the "never ran" fixture above: once something (a
        // future `WS4` executor) has written a real `last_run_at`/
        // `freshness_lag_seconds`, `Pipeline::from` must surface it — it is
        // no longer a placeholder to override. `sla_ok` still is.
        let ran_at = OffsetDateTime::from_unix_timestamp(1_735_689_600).unwrap();
        let row = PipelineRow {
            id: "pl-x".to_owned(),
            name: "n".to_owned(),
            kind: "batch".to_owned(),
            status: "ready".to_owned(),
            owner: "o".to_owned(),
            source: "s".to_owned(),
            target: "t".to_owned(),
            connector_id: None,
            source_asset_id: None,
            target_asset_id: None,
            schedule: "manual".to_owned(),
            last_run_at: Some(ran_at),
            next_run_at: None,
            freshness_lag_seconds: Some(42),
            max_retries: 3,
            description: Some("does a thing".to_owned()),
            depends_on: vec!["pl-up".to_owned(), "ingest_job".to_owned()],
        };
        let pipeline = Pipeline::from(row);
        assert_eq!(pipeline.last_run_at, Some(iso_millis(ran_at)));
        assert_eq!(pipeline.sla_ok, None);
        assert_eq!(pipeline.freshness_lag_seconds, Some(42));
        assert_eq!(pipeline.description.as_deref(), Some("does a thing"));
        assert_eq!(
            pipeline.depends_on,
            vec!["pl-up".to_owned(), "ingest_job".to_owned()],
            "depends_on must round-trip the text[] column verbatim",
        );
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

    #[test]
    fn pipeline_sla_serializes_camel_case_with_null_for_unset_thresholds() {
        // Mirrors `DatasetSla` in `contracts/pipelines.ts`: the GET
        // response shape must surface both thresholds as nullable so the
        // UI can render `maxDurationSeconds: null` rather than `0`. A
        // `Some(0)` would be an "always over" SLA and the route rejects
        // it; this fixture just proves the NULL case serializes cleanly.
        let sla = PipelineSla {
            pipeline_id: "silver_orders".to_owned(),
            max_duration_seconds: None,
            late_after_seconds: Some(3_600),
            updated_by: Uuid::from_u128(1),
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
        };
        let value = serde_json::to_value(&sla).unwrap();
        assert_eq!(value["pipelineId"], "silver_orders");
        assert!(value["maxDurationSeconds"].is_null());
        assert_eq!(value["lateAfterSeconds"], 3_600);
        assert_eq!(value["updatedBy"], Uuid::from_u128(1).to_string());
        assert_eq!(value["updatedAt"], "2026-01-01T00:00:00.000Z");
    }
}
