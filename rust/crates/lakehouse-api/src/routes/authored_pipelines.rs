//! Console-authored pipelines (`pl-*`) as real orchestrator jobs.
//!
//! # How an authored pipeline runs
//!
//! `dagster/dispar_orchestrate/authored_factory.py` builds one job
//! (`authored__<id>`) and, when the authored schedule is a cron, one
//! schedule (`authored__<id>_schedule`) per pipeline in status `ready` or
//! `paused`. It reads them from [`runnable`] when the code location is
//! imported. So the orchestrator only learns about a change when the code
//! location reloads: every write here that changes what should exist (mark
//! ready, edit, delete, pause, resume) ends by asking it to reload
//! ([`reload_orchestrator`]). The code location runs `dagster code-server
//! start`, which re-imports its module on reload; the plain `dagster api
//! grpc` server it used before would not have.
//!
//! # Who may read every tenant's definitions
//!
//! [`runnable`] lists authored pipelines across all tenants, as
//! `GET /api/connectors/ingestible` does for connectors, because the
//! orchestrator runs every tenant's work. The policy floor is
//! `pipeline:read`, which people hold too, so the handler additionally
//! refuses anyone who is neither a service identity nor a Platform Admin
//! (`*:*`, who already reads across tenants: `catalog::is_unrestricted`,
//! the same rule the shared catalog applies). Anyone else reading it would
//! see other tenants' pipelines.
//!
//! # What reaches a response
//!
//! A reload failure is the orchestrator's own text (often a Python
//! traceback). It is logged and never returned; the response says only
//! whether the orchestrator picked the change up (AGENTS.md principle 4).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_store::pipelines::{self, UpdatePipelineInput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::pipelines::record_pipeline_audit;
use crate::state::AppState;

/// Ticks read per request: enough to see a day of a 15-minute schedule.
const MAX_TICKS: u32 = 50;

/// The Dagster job name `authored_factory.py` gives pipeline `id`:
/// `authored__` plus the id with every character outside `[A-Za-z0-9_]`
/// replaced by `_` (Python's `_dagster_safe_name`; ids are ASCII slugs, so
/// `str.isalnum` and `char::is_ascii_alphanumeric` agree on them).
#[must_use]
pub fn job_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("authored__{safe}")
}

/// The schedule name `authored_factory.py` gives pipeline `id`'s job.
#[must_use]
pub fn schedule_name(id: &str) -> String {
    format!("{}_schedule", job_name(id))
}

/// The dependency-sensor name `authored_factory.py` gives pipeline `id`
/// when `id` has `depends_on` populated. The Python factory builds one
/// `run_status_sensor` per downstream pipeline with non-empty
/// `depends_on`, named `authored__<safe_id>_after` so the same sanitize
/// rule that gives `job_name` / `schedule_name` matches here. R3 plan 2a.
#[must_use]
pub fn sensor_name(id: &str) -> String {
    format!("{}_after", job_name(id))
}

/// Maximum number of upstream pipelines a downstream may declare. The
/// chosen cap is small enough that an ALL-semantics sensor's per-tick
/// freshness walk stays cheap (10 sequential `instance.get_runs` calls
/// against a local SQLite/Postgres instance — see the dependency sensor
/// in `dagster/dispar_orchestrate/authored_factory.py`) and large enough
/// that no realistic "Bronze → Silver → Gold" chain in this product
/// needs to argue the case with the validator.
pub const MAX_DEPENDS_ON: usize = 10;

/// Load the `(id, depends_on)` for every authored pipeline so
/// [`validate_depends_on`] can run its cycle walk. One query, regardless
/// of how many pipelines exist — the walk is the cheap part, the fetch
/// is the bounded part.
///
/// `exclude_id` lets the create/update paths omit the row they are about
/// to write from the lookup (the validator substitutes the proposed
/// `depends_on` for `exclude_id`); passing `None` includes every row,
/// which is what the detail route uses to compute `downstream`.
pub async fn collect_authored_depends_on(
    pool: &lakehouse_store::PgPool,
    exclude_id: Option<&str>,
) -> Result<Vec<(String, Vec<String>)>, ApiRejection> {
    // Routes this function is called from already wrapped the 503/500
    // path in `crate::routes::pipelines::pool(&state)?`. A real failure
    // here would be a database outage, classified through the same
    // [`lakehouse_store::StoreError`] -> [`ApiError`] mapping every
    // other store call uses (AGENTS.md rule 4: never leak upstream text
    // into a response).
    let rows: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT id, depends_on FROM pipeline_definition \
         WHERE ($1::text IS NULL OR id <> $1) ORDER BY created_at",
    )
    .bind(exclude_id)
    .fetch_all(pool)
    .await
    .map_err(lakehouse_store::StoreError::from)?;
    Ok(rows)
}

/// Validate `new_depends_on` for pipeline `this_id` against the existing
/// authored graph and the orchestrator's live Dagster jobs. Pure
/// function — every input is passed in by the caller, no I/O here.
///
/// Returns `Ok(())` when the proposed set is acceptable; the route turns
/// `Err(ApiRejection)` into a 400 with the validator's own message,
/// which names the offending id verbatim.
///
/// Rules (in this order, so the cheapest rejection wins):
///
/// 1. No self-reference (`this_id` listed in `new_depends_on`).
/// 2. Each id is an authored pipeline that currently exists in `others`
///    or a Dagster job the orchestrator lists in `dagster_jobs`.
/// 3. At most [`MAX_DEPENDS_ON`] entries.
/// 4. No cycle across authored pipelines (DFS over authored `depends_on`):
///    starting from any of `new_depends_on`'s ids, the walk must never
///    reach `this_id`. The walk treats `this_id` as having
///    `new_depends_on` (the proposed set), not its previous value, so a
///    sequence like `A → B`, `B → C`, `C → A` is caught when the user
///    submits the third edit — not silently allowed by reading the
///    pre-update graph.
pub fn validate_depends_on(
    this_id: &str,
    new_depends_on: &[String],
    others: &[(String, Vec<String>)],
    dagster_jobs: &[String],
) -> Result<(), ApiRejection> {
    // Rule 1: self-reference. Not a cycle in the graph-theory sense (it
    // terminates at `this_id`), but a pipeline that depends on itself is
    // semantically broken: the sensor would fire only after a SUCCESS
    // run of itself, which can never start until the sensor fires, so
    // the chain deadlocks forever.
    if new_depends_on.iter().any(|d| d == this_id) {
        return Err(ApiError::BadRequest(format!(
            "depends_on must not include the pipeline's own id ({this_id:?})"
        ))
        .into());
    }

    // Rule 3: count cap. Checked before "is every id valid" so an over-cap
    // submission always gets the same error shape regardless of which id
    // is invalid — the route does not echo caller input, but the count
    // cap is a stable invariant every caller can compute from their own
    // input.
    if new_depends_on.len() > MAX_DEPENDS_ON {
        return Err(ApiError::BadRequest(format!(
            "depends_on has {} entries, the maximum is {MAX_DEPENDS_ON}",
            new_depends_on.len()
        ))
        .into());
    }

    // Rule 2: every id resolves to an authored pipeline or a Dagster
    // job. `others` is the full authored set with the caller's prior
    // `depends_on` for this_id (already replaced by `new_depends_on` —
    // see `validate_no_cycle`); the lookup is a linear scan because
    // `others` has at most a few dozen entries and the validator is
    // called once per write.
    for dep in new_depends_on {
        let is_authored = others.iter().any(|(id, _)| id == dep);
        let is_dagster = dagster_jobs.iter().any(|j| j == dep);
        if !is_authored && !is_dagster {
            return Err(ApiError::BadRequest(format!(
                "depends_on references unknown pipeline {dep:?} \
                 (must be an existing authored pipeline id or a Dagster job name)"
            ))
            .into());
        }
    }

    // Rule 4: cycle detection. Build an "effective" view of the graph
    // where `this_id`'s `depends_on` is the proposed new set, then DFS
    // from every proposed upstream; any path that reaches `this_id` is a
    // cycle (a→b→…→a).
    validate_no_cycle(this_id, new_depends_on, others)
}

/// Reverse-reference lookup for the delete guard: every authored
/// pipeline whose `depends_on` lists `target_id`. Used by
/// [`delete`] to refuse removal of an upstream that another pipeline
/// still references (final-review fix, "dangling `depends_on` after
/// upstream deletion"). Pure function, no I/O — extracted from the
/// route so the reverse walk is unit-tested directly: the route
/// itself cannot be tested at this seam without a real pool (the
/// existing `state_without_pool()` test fixture returns 503 before
/// any guard runs), so the guard's correctness lives here.
///
/// Order is preserved from `pairs` (the SQL query behind
/// [`collect_authored_depends_on`] returns `ORDER BY created_at`,
/// which is also the order the user sees them in the UI's pipeline
/// list); a stable, predictable order makes the 400 message
/// diff-friendly across runs.
#[must_use]
pub fn referencing_downstreams(pairs: &[(String, Vec<String>)], target_id: &str) -> Vec<String> {
    pairs
        .iter()
        .filter(|(_, deps)| deps.contains(&target_id.to_owned()))
        .map(|(id, _)| id.clone())
        .collect()
}

/// DFS over the authored `depends_on` graph with `this_id`'s edges
/// overridden to `new_depends_on`. Pure function, no I/O — extracted
/// from [`validate_depends_on`] so the cycle rule has its own unit
/// tests (the 3-cycle fixture is the regression that proves the
/// "treat the edited row as having the proposed edges" rule is
/// applied at the cycle walk, not at the input check).
///
/// Dagster-native jobs are excluded from the walk: they have no
/// declared `depends_on` and cannot, therefore, be part of a cycle —
/// including them here would either over-restrict or under-restrict
/// depending on whether the walk treated their empty `depends_on` as
/// "dead end" or "self".
fn validate_no_cycle(
    this_id: &str,
    new_depends_on: &[String],
    others: &[(String, Vec<String>)],
) -> Result<(), ApiRejection> {
    fn visit(
        current: &str,
        this_id: &str,
        new_depends_on: &[String],
        others: &[(String, Vec<String>)],
        on_stack: &mut Vec<String>,
    ) -> Result<(), String> {
        if on_stack.iter().any(|n| n == current) {
            // `current` is being visited on the current DFS path.
            // When `current == this_id`, the cycle closes back to the
            // pipeline being edited — the exact condition we are
            // guarding against.
            return Err(current.to_owned());
        }
        on_stack.push(current.to_owned());
        // The current node's outgoing edges: the NEW set for `this_id`,
        // every other pipeline's stored set.
        let edges: Vec<String> = if current == this_id {
            new_depends_on.to_vec()
        } else {
            others
                .iter()
                .find(|(id, _)| id == current)
                .map(|(_, deps)| deps.clone())
                .unwrap_or_default()
        };
        for next in edges {
            // Skip Dagster jobs — they cannot be part of a cycle (no
            // `depends_on`), so walking them is wasted work that would
            // also treat their empty edge list as a terminal node and
            // never report a cycle that actually terminates in them.
            if others.iter().all(|(id, _)| id != &next) {
                continue;
            }
            visit(&next, this_id, new_depends_on, others, on_stack)?;
        }
        on_stack.pop();
        Ok(())
    }
    let mut on_stack = Vec::new();
    for start in new_depends_on {
        visit(start, this_id, new_depends_on, others, &mut on_stack).map_err(
            |cycle_back_to| -> ApiRejection {
                ApiError::BadRequest(format!(
                    "depends_on would create a cycle back to pipeline {cycle_back_to:?}"
                ))
                .into()
            },
        )?;
    }
    Ok(())
}

/// Ask the orchestrator to rebuild its authored jobs. Best effort: the
/// write that called this has already succeeded, and a failed reload is
/// reported as `false`, never as the write failing. The reason is logged,
/// not returned.
pub async fn reload_orchestrator(state: &AppState) -> bool {
    match state.dagster.reload_location().await {
        Ok(outcome) => match outcome.error {
            None => true,
            Some(err) => {
                tracing::warn!(%err, "authored pipelines: orchestrator reload was refused");
                false
            }
        },
        Err(err) => {
            tracing::warn!(%err, "authored pipelines: orchestrator unreachable for reload");
            false
        }
    }
}

/// Whether the orchestrator has a job for pipeline `id` right now.
pub async fn job_is_loaded(state: &AppState, id: &str) -> bool {
    let name = job_name(id);
    state
        .dagster
        .list_jobs()
        .await
        .is_ok_and(|jobs| jobs.contains(&name))
}

/// `GET /api/pipelines/runnable` — every `ready`/`paused` authored
/// pipeline with its definition, for the orchestrator's job factory.
///
/// # Errors
///
/// 403 for anyone but a service identity or a Platform Admin (see the
/// module doc); 503 with
/// no database; 500 on a database failure.
pub async fn runnable(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity reads every tenant's pipelines".to_owned(),
        )
        .into());
    }
    let pool = crate::routes::pipelines::pool(&state)?;
    let list = pipelines::list_runnable_pipelines(pool).await?;
    Ok(ApiJson(json!({ "pipelines": list })))
}

/// `PUT /api/pipelines/{id}` body: an authored pipeline's editable fields.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    kind: String,
    source_zone: String,
    source_table: String,
    #[serde(default)]
    incremental_column: Option<String>,
    #[serde(default)]
    transforms: Vec<String>,
    #[serde(default)]
    fbic_enabled: bool,
    target_zone: String,
    target_table: String,
    schedule: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    description: Option<String>,
    /// Plan 1c: per-pipeline retry cap (`max_retries`, migration 0051).
    /// Validated here against `0..=5` — the same range the column CHECK
    /// and `dagster.RetryPolicy.max_retries` accept — so the client
    /// receives a 400 with the field name rather than a 500 from the
    /// database constraint. `None` leaves the stored value alone
    /// (`UpdatePipelineInput::max_retries` is "unchanged", matching
    /// `owner`/`description`/`incremental_column`'s "absent means
    /// leave it alone" convention).
    #[serde(default)]
    max_retries: Option<i16>,
    /// Upstream pipeline ids (authored `pl-…` or Dagster job names). R3
    /// plan 2a, migration `0052`. Empty list when omitted, matching the
    /// column's `DEFAULT '{}'`. The route validates every id and the
    /// resulting graph BEFORE calling the store (see
    /// [`validate_depends_on`]); an invalid list is refused with 400
    /// and writes nothing.
    #[serde(default)]
    depends_on: Vec<String>,
}

fn authored_only(id: &str) -> Result<(), ApiRejection> {
    if id.starts_with("pl-") {
        Ok(())
    } else {
        // A Dagster job is defined in code; it is edited there, not here.
        Err(ApiError::NotFound(format!("Pipeline {id} not found")).into())
    }
}

/// The body every authored write returns: the pipeline, plus whether the
/// orchestrator picked the change up.
fn with_orchestrator(pipeline: &pipelines::Pipeline, reloaded: bool) -> Value {
    let mut body = serde_json::to_value(pipeline).unwrap_or_else(|_| json!({}));
    if let Value::Object(obj) = &mut body {
        obj.insert("orchestratorReloaded".to_owned(), json!(reloaded));
    }
    body
}

/// `PUT /api/pipelines/{id}` — replace an authored pipeline's definition.
/// Transforms go through the same grammar as `POST /api/pipelines`.
///
/// # Errors
///
/// 404 for an unknown or non-authored id; 400 for a malformed body or a
/// transform outside the grammar; 503/500 from the store.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    authored_only(&id)?;
    let body: UpdateBody = crate::routes::pipelines::parse_body(&body)?;
    for (index, transform) in body.transforms.iter().enumerate() {
        crate::transform_grammar::parse_transform(transform).map_err(|err| {
            ApiError::BadRequest(format!("invalid transform at transforms[{index}]: {err}"))
        })?;
    }
    // Plan 1c: validate `max_retries` against the same `0..=5` the
    // column CHECK enforces (migration 0051), so the client gets a 400
    // with the field name rather than a 500 from the database
    // constraint. The error message does not echo the caller's value
    // back.
    if let Some(value) = body.max_retries
        && !(0..=5).contains(&value)
    {
        return Err(ApiError::BadRequest("maxRetries must be between 0 and 5".to_owned()).into());
    }
    let pool = crate::routes::pipelines::pool(&state)?;
    // Validate `depends_on` against the existing authored graph and the
    // live Dagster job list — the pipeline itself does not yet have to
    // exist for validation (the create-time path runs before the row is
    // committed; the update path runs against its current state). Both
    // paths refuse an invalid set with 400 BEFORE the store is touched.
    //
    // Dagster unreachable degrades to "no Dagster upstreams accepted"
    // (empty list) rather than 500ing the update — see
    // `routes::pipelines::create`'s identical treatment. The rule
    // "no-cycle through authored pipelines" still fires; the rule
    // "every id is a real Dagster job" cannot, so a submitted Dagster
    // name that does not match the (unreachable) live list is treated
    // as if no live list exists, which means the submitter will see
    // "unknown pipeline" once Dagster is back and the rule fires again.
    let dagster_jobs = match state.dagster.list_jobs().await {
        Ok(j) => j,
        Err(err) => {
            tracing::warn!(%err, "update: dagster unreachable, depends_on accepts authored upstreams only");
            Vec::new()
        }
    };
    let others = collect_authored_depends_on(pool, Some(&id)).await?;
    validate_depends_on(&id, &body.depends_on, &others, &dagster_jobs)?;
    let input = UpdatePipelineInput {
        kind: body.kind,
        source_zone: body.source_zone,
        source_table: body.source_table,
        incremental_column: body.incremental_column.filter(|c| !c.trim().is_empty()),
        transforms: body.transforms,
        fbic_enabled: body.fbic_enabled,
        target_zone: body.target_zone,
        target_table: body.target_table,
        schedule: body.schedule,
        owner: body.owner.filter(|o| !o.trim().is_empty()),
        description: body.description.filter(|d| !d.trim().is_empty()),
        max_retries: body.max_retries,
        depends_on: body.depends_on,
    };
    let updated = pipelines::update_pipeline(pool, &id, &input, Some(principal.id.uuid()))
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} not found")))?;
    record_pipeline_audit(&state, &principal, "pipeline.update", &id, Value::Null).await;
    // A draft has no job; only a runnable pipeline's job has to be rebuilt.
    let reloaded = if updated.status == "draft" {
        false
    } else {
        reload_orchestrator(&state).await
    };
    Ok(ApiJson(with_orchestrator(&updated, reloaded)))
}

/// `DELETE /api/pipelines/{id}` — delete an authored pipeline. Its past
/// runs stay in the orchestrator's history; its job and schedule go away
/// on the reload that follows.
///
/// Refused with 400 when another authored pipeline lists `id` in its
/// `depends_on` — that downstream keeps the id in its graph, so
/// deleting the upstream would leave a dangling reference the validator
/// refuses the next time the downstream is edited, AND a sensor that
/// watches a job that no longer exists. Better to fail closed and let
/// the author remove the references first (final-review fix, "dangling
/// `depends_on` after upstream deletion"). The 400 names every
/// downstream id verbatim so the UI can highlight what to edit.
///
/// # Errors
///
/// 404 for an unknown or non-authored id; 400 when `id` is still
/// referenced as an upstream by another authored pipeline; 503/500
/// from the store.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Response {
    if let Err(rejection) = authored_only(&id) {
        return rejection.into_response();
    }
    let pool = match crate::routes::pipelines::pool(&state) {
        Ok(pool) => pool,
        Err(err) => return ApiRejection(err).into_response(),
    };
    // `exclude_id = Some(&id)` skips the row being deleted (which
    // cannot list itself anyway — `validate_depends_on` already
    // refuses a self-reference — but excluding it costs nothing and
    // keeps the query bounded).
    let others = match collect_authored_depends_on(pool, Some(&id)).await {
        Ok(others) => others,
        Err(err) => return err.into_response(),
    };
    let referencing = referencing_downstreams(&others, &id);
    if !referencing.is_empty() {
        return ApiRejection(ApiError::BadRequest(format!(
            "cannot delete pipeline {id:?}: it is still referenced as an \
             upstream by {} other pipeline(s): {referencing:?}; remove \
             those references before deleting",
            referencing.len()
        )))
        .into_response();
    }
    match pipelines::delete_pipeline(pool, &id, Some(principal.id.uuid())).await {
        Ok(true) => {
            record_pipeline_audit(&state, &principal, "pipeline.delete", &id, Value::Null).await;
            let reloaded = reload_orchestrator(&state).await;
            (
                StatusCode::OK,
                ApiJson(json!({ "id": id, "deleted": true, "orchestratorReloaded": reloaded })),
            )
                .into_response()
        }
        Ok(false) => {
            ApiRejection(ApiError::NotFound(format!("Pipeline {id} not found"))).into_response()
        }
        Err(err) => ApiRejection(err.into()).into_response(),
    }
}

/// `POST /api/pipelines/{id}/versions/{version}/restore` — replay a
/// stored snapshot back into the live row (Plan R4 2b). Reads the
/// snapshot via `pipelines::get_definition_version`, rebuilds an
/// `UpdatePipelineInput` from it (everything except `name` and
/// `status`, which the spec excludes from restore), and goes through
/// `pipelines::restore_pipeline` so the write is wrapped in a single
/// tx that also captures the resulting version row with `event =
/// "restored"` — the governance trail records the restore as a
/// distinct event from a plain `update`.
///
/// The restore does NOT change `name` or `status`: renaming or
/// re-promoting a pipeline is a separate, deliberate action. A
/// restore of the original `created` snapshot on a currently-`paused`
/// pipeline leaves the pipeline paused.
///
/// The orchestrator is reloaded on success, like every other
/// definition-mutating route, because the rebuilt job may differ
/// from the one currently running.
///
/// # Errors
///
/// 404 for a non-`pl-` id or for an unknown `(id, version)` pair;
/// 500/503 from the store or the orchestrator.
pub async fn restore_version(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((id, version)): Path<(String, i32)>,
) -> ApiResult<ApiJson<Value>> {
    authored_only(&id)?;
    let pool = crate::routes::pipelines::pool(&state)?;
    let snapshot = pipelines::get_definition_version(pool, &id, version)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} version {version} not found")))?;
    // Validate the snapshot's `depends_on` against the live authored
    // graph and the orchestrator's `Dagster` job list — the same
    // `validate_depends_on` call `update` runs (final-review
    // should-fix, part 2b: a stored snapshot can outlive one of its
    // declared upstreams; restoring it would persist a dangling
    // reference identical to the one the delete guard refuses to
    // create on the live graph — `referencing_downstreams`, same
    // file). The snapshot itself cannot list `id`: every stored
    // version was written through a path the validator had already
    // cleared (or, for the original `created` snapshot, validated
    // before insert). `exclude_id = Some(&id)` is belt-and-braces,
    // same call shape as `update`. `Dagster` unreachable degrades
    // to "no `Dagster` upstreams accepted" (empty list), identical
    // to `update`'s fallback. After a successful restore the
    // persisted state satisfies the validator by construction: we
    // just validated the exact set we are about to write.
    let dagster_jobs = match state.dagster.list_jobs().await {
        Ok(j) => j,
        Err(err) => {
            tracing::warn!(%err, "restore_version: dagster unreachable, depends_on accepts authored upstreams only");
            Vec::new()
        }
    };
    let others = collect_authored_depends_on(pool, Some(&id)).await?;
    validate_depends_on(&id, &snapshot.depends_on, &others, &dagster_jobs)?;
    // Rebuild an `UpdatePipelineInput` from the snapshot. `name` and
    // `status` are deliberately excluded: a restore is a replay of the
    // editable fields, not a rename/re-promotion (Plan R4 2b).
    let input = UpdatePipelineInput {
        kind: snapshot.kind,
        source_zone: snapshot.source_zone,
        source_table: snapshot.source_table,
        incremental_column: snapshot.incremental_column,
        transforms: snapshot.transforms,
        fbic_enabled: snapshot.fbic_enabled,
        target_zone: snapshot.target_zone,
        target_table: snapshot.target_table,
        schedule: snapshot.schedule,
        owner: Some(snapshot.owner),
        description: snapshot.description,
        max_retries: Some(snapshot.max_retries),
        depends_on: snapshot.depends_on,
    };
    let updated = pipelines::restore_pipeline(pool, &id, &input, Some(principal.id.uuid()))
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} not found")))?;
    record_pipeline_audit(
        &state,
        &principal,
        &format!("pipeline.restore.{version}"),
        &id,
        Value::Null,
    )
    .await;
    // Drafts have no job; only runnable pipelines need a reload.
    let reloaded = if updated.status == "draft" {
        false
    } else {
        reload_orchestrator(&state).await
    };
    Ok(ApiJson(with_orchestrator(&updated, reloaded)))
}

/// `GET /api/pipelines/{id}/schedule-ticks` — the schedule's recent
/// evaluations, newest first. A Dagster job's schedule is its first one
/// (the same rule `schedule_label` follows); an authored pipeline's is
/// `authored__<id>_schedule`. R3 plan 2a also merges in the
/// `authored__<id>_after` sensor's ticks when the pipeline has
/// `depends_on`; each tick is tagged `kind:"schedule"` or `kind:"sensor"`
/// so the UI can render the dependency sensor's `SkipReason` separately
/// from the schedule's launch / skip / fail outcomes. No schedule reads
/// as an empty list with `schedule: null`; an unreachable orchestrator as
/// `unavailable`.
pub async fn schedule_ticks(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let schedule = if id.starts_with("pl-") {
        Some(schedule_name(&id))
    } else {
        match state.dagster.list_jobs_with_schedules().await {
            Ok(jobs) => jobs
                .iter()
                .find(|j| j.name == id)
                .and_then(|j| j.schedules.first())
                .map(|s| s.name.clone()),
            Err(err) => {
                tracing::warn!(%err, "schedule ticks: orchestrator unreachable");
                return unavailable();
            }
        }
    };
    let Some(schedule) = schedule else {
        return (
            StatusCode::OK,
            ApiJson(json!({ "schedule": Value::Null, "ticks": [], "unavailable": Value::Null })),
        )
            .into_response();
    };
    // Fetch the schedule ticks and, for authored pipelines with
    // `depends_on`, the dependency sensor ticks in parallel — the UI
    // would otherwise show "no upstream checks" until the schedule's
    // own ticks returned. A missing sensor (no `depends_on`, or one not
    // yet rebuilt) is normal: empty list, no warning.
    let is_authored = id.starts_with("pl-");
    let sensor = is_authored.then(|| sensor_name(&id));
    let (schedule_ticks, sensor_ticks) =
        tokio::join!(state.dagster.schedule_ticks(&schedule, MAX_TICKS), async {
            match sensor {
                Some(name) => state
                    .dagster
                    .sensor_ticks(&name, MAX_TICKS)
                    .await
                    .map_err(|err| {
                        tracing::warn!(
                            %err,
                            sensor = %name,
                            "schedule ticks: orchestrator sensor unreachable"
                        );
                        err
                    })
                    .unwrap_or_default(),
                None => Vec::new(),
            }
        },);
    let mut ticks: Vec<Value> =
        Vec::with_capacity(schedule_ticks.as_ref().map_or(0, Vec::len) + sensor_ticks.len());
    match schedule_ticks {
        Ok(t) => ticks.extend(t.into_iter().map(|t| {
            let mut v = serde_json::to_value(&t).unwrap_or_else(|_| json!({}));
            if let Value::Object(obj) = &mut v {
                obj.insert("kind".to_owned(), json!("schedule"));
            }
            v
        })),
        Err(err) => {
            tracing::warn!(%err, "schedule ticks: orchestrator unreachable");
            return unavailable();
        }
    }
    // Sensor ticks already carry `kind:"sensor"` in the merged shape;
    // merge in the rest, each tagged so the UI can distinguish them.
    ticks.extend(sensor_ticks.into_iter().map(|t| {
        let mut v = serde_json::to_value(&t).unwrap_or_else(|_| json!({}));
        if let Value::Object(obj) = &mut v {
            obj.insert("kind".to_owned(), json!("sensor"));
        }
        v
    }));
    (
        StatusCode::OK,
        ApiJson(json!({ "schedule": schedule, "ticks": ticks, "unavailable": Value::Null })),
    )
        .into_response()
}

fn unavailable() -> Response {
    (
        StatusCode::OK,
        ApiJson(json!({
            "schedule": Value::Null,
            "ticks": [],
            "unavailable": "the orchestrator could not be reached",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    fn principal(id: PrincipalId) -> Principal {
        Principal {
            id,
            tenant_ids: Vec::new(),
            display_name: "fixture".to_owned(),
            permissions: PermissionSet::parse("pipeline:read pipeline:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    #[test]
    fn job_and_schedule_names_match_the_python_factory() {
        // `authored_factory._dagster_safe_name("pl-ui-check-flow-mumidzub")`.
        assert_eq!(
            job_name("pl-ui-check-flow-mumidzub"),
            "authored__pl_ui_check_flow_mumidzub"
        );
        assert_eq!(schedule_name("pl-a-1"), "authored__pl_a_1_schedule");
    }

    /// R3 plan 2a: a 3-cycle `A → B → C → A` is caught when the user
    /// edits C's `depends_on` to `[A]`, not silently allowed because the
    /// pre-edit `depends_on` of C is read instead. The cycle walk treats
    /// the edited row as having the proposed edges; this is the
    /// regression that proves the override is at the cycle walk, not at
    /// the input check.
    #[test]
    fn validate_depends_on_detects_a_three_cycle_when_editing_the_closing_edge() {
        // Pre-edit authored state: A→B, B→C; C has no `depends_on` yet.
        let others = vec![
            ("pl-a".to_owned(), vec!["pl-b".to_owned()]),
            ("pl-b".to_owned(), vec!["pl-c".to_owned()]),
            ("pl-c".to_owned(), Vec::new()),
        ];
        let dagster = vec!["ingest_job".to_owned()];
        // The edit: C depends on A → A→B→C→A is a cycle. The walk
        // returns the cycle's entry node (A, which the DFS first
        // visited), not the edited pipeline (C, which is on the cycle
        // path but not the node the walk re-enters through).
        let err = validate_depends_on("pl-c", &["pl-a".to_owned()], &others, &dagster).unwrap_err();
        let message = err.0.to_string();
        assert!(
            message.contains("cycle"),
            "error must name the rule that fired: {message:?}"
        );
        assert!(
            message.contains("pl-a"),
            "error must name the cycle's entry node (the first node visited twice): {message:?}"
        );
    }

    /// A pipeline that lists itself in its own `depends_on` is refused
    /// before the cycle walk, with a message that names the offending
    /// id (not a generic "invalid input" — see the route's 400 contract
    /// for cycle/unknown-id/cap cases, all of which name the id).
    #[test]
    fn validate_depends_on_refuses_self_reference() {
        let others = vec![("pl-a".to_owned(), Vec::new())];
        let dagster: Vec<String> = Vec::new();
        let err = validate_depends_on("pl-a", &["pl-a".to_owned()], &others, &dagster).unwrap_err();
        let message = err.0.to_string();
        assert!(
            message.contains("own id") && message.contains("pl-a"),
            "self-reference must be named plainly: {message:?}"
        );
    }

    /// An upstream id that names neither an authored pipeline nor a
    /// Dagster job is refused with a 400 that quotes the offending id
    /// verbatim (the route's contract — see the spec for the "400 naming
    /// the offending id" requirement).
    #[test]
    fn validate_depends_on_refuses_unknown_id_naming_it() {
        let others = vec![("pl-a".to_owned(), Vec::new())];
        let dagster = vec!["ingest_job".to_owned()];
        let err =
            validate_depends_on("pl-a", &["pl-ghost".to_owned()], &others, &dagster).unwrap_err();
        let message = err.0.to_string();
        assert!(
            message.contains("pl-ghost"),
            "unknown id must appear verbatim in the error: {message:?}"
        );
    }

    /// At most [`MAX_DEPENDS_ON`] upstreams. The cap is the cheap,
    /// upfront rejection — tested here against the boundary (`>10`) so
    /// the validator's behavior at the limit is also exercised below.
    #[test]
    fn validate_depends_on_refuses_more_than_max_upstreams() {
        let others = vec![("pl-a".to_owned(), Vec::new())];
        let dagster: Vec<String> = Vec::new();
        let too_many: Vec<String> = (0..=MAX_DEPENDS_ON).map(|i| format!("pl-up-{i}")).collect();
        let err = validate_depends_on("pl-a", &too_many, &others, &dagster).unwrap_err();
        let message = err.0.to_string();
        assert!(
            message.contains(&format!("maximum is {MAX_DEPENDS_ON}")),
            "cap error must name the limit: {message:?}"
        );
    }

    /// Exactly `MAX_DEPENDS_ON` upstreams is accepted — the boundary
    /// belongs to `MAX_DEPENDS_ON`, not to `< MAX_DEPENDS_ON - 1`.
    #[test]
    fn validate_depends_on_accepts_exactly_max_upstreams() {
        let mut others: Vec<(String, Vec<String>)> = (0..MAX_DEPENDS_ON)
            .map(|i| (format!("pl-up-{i}"), Vec::<String>::new()))
            .collect();
        others.push(("pl-a".to_owned(), Vec::new()));
        let at_limit: Vec<String> = (0..MAX_DEPENDS_ON).map(|i| format!("pl-up-{i}")).collect();
        assert!(
            validate_depends_on("pl-a", &at_limit, &others, &[]).is_ok(),
            "exactly {MAX_DEPENDS_ON} upstreams must be accepted"
        );
    }

    /// A Dagster-native upstream is accepted alongside authored ones; the
    /// cycle walk skips Dagster jobs because they have no `depends_on`
    /// (they cannot close a cycle).
    #[test]
    fn validate_depends_on_accepts_a_dagster_native_upstream() {
        let others = vec![("pl-a".to_owned(), Vec::new())];
        let dagster = vec!["ingest_job".to_owned()];
        assert!(
            validate_depends_on("pl-a", &["ingest_job".to_owned()], &others, &dagster).is_ok(),
            "a Dagster job from list_jobs must be a valid upstream"
        );
    }

    /// A chain `A → B → C` (C depends on nothing yet, A depends on B,
    /// B depends on C) is acyclic when the user edits A's `depends_on`
    /// to `[B, ingest_job]` — the validator must not report a cycle.
    /// This is the negative test paired with the 3-cycle case above:
    /// the validator doesn't false-positive on every DAG that happens
    /// to mention C twice.
    #[test]
    fn validate_depends_on_accepts_a_valid_chain() {
        let others = vec![
            ("pl-b".to_owned(), vec!["pl-c".to_owned()]),
            ("pl-c".to_owned(), Vec::new()),
        ];
        let dagster = vec!["ingest_job".to_owned()];
        assert!(
            validate_depends_on(
                "pl-a",
                &["pl-b".to_owned(), "ingest_job".to_owned()],
                &others,
                &dagster,
            )
            .is_ok(),
            "A -> [B, ingest_job] with B -> C is not a cycle"
        );
    }

    /// The delete guard refuses removal of an upstream that is still
    /// referenced. With no downstreams, the helper returns an empty
    /// list — the guard would let the delete through. Without this
    /// negative case the helper would never be proven to handle "no
    /// match" cleanly (a helper that always returned empty would pass
    /// no test until the multi-downstream case was added).
    #[test]
    fn referencing_downstreams_returns_empty_when_no_pipeline_references_the_target() {
        let others = vec![
            ("pl-a".to_owned(), Vec::new()),
            ("pl-b".to_owned(), vec!["pl-c".to_owned()]),
        ];
        assert!(
            referencing_downstreams(&others, "pl-zzz").is_empty(),
            "no pipeline lists pl-zzz in its depends_on"
        );
    }

    /// A single pipeline that lists the target in its `depends_on`
    /// is returned verbatim. This is the basic positive case the
    /// delete guard's 400 message will quote.
    #[test]
    fn referencing_downstreams_returns_a_pipeline_that_depends_on_the_target() {
        let others = vec![
            ("pl-a".to_owned(), vec!["pl-up".to_owned()]),
            ("pl-b".to_owned(), Vec::new()),
        ];
        assert_eq!(
            referencing_downstreams(&others, "pl-up"),
            vec!["pl-a".to_owned()],
            "only pl-a lists pl-up in its depends_on"
        );
    }

    /// Several pipelines may reference the same upstream (a "fan-in"
    /// pattern); the helper returns every downstream in input order,
    /// which is `ORDER BY created_at` from the SQL behind
    /// [`collect_authored_depends_on`]. Stable order keeps the 400
    /// message diff-friendly across runs.
    #[test]
    fn referencing_downstreams_returns_every_referencing_pipeline_in_input_order() {
        let others = vec![
            ("pl-down-1".to_owned(), vec!["pl-up".to_owned()]),
            ("pl-down-2".to_owned(), vec!["pl-up".to_owned()]),
            ("pl-other".to_owned(), vec!["pl-elsewhere".to_owned()]),
            (
                "pl-down-3".to_owned(),
                vec!["pl-up".to_owned(), "pl-other".to_owned()],
            ),
        ];
        assert_eq!(
            referencing_downstreams(&others, "pl-up"),
            vec![
                "pl-down-1".to_owned(),
                "pl-down-2".to_owned(),
                "pl-down-3".to_owned(),
            ],
            "three downstreams list pl-up; pl-other does not"
        );
    }

    /// A pipeline that lists the target alongside other upstreams
    /// (its own fan-in graph) still counts as a downstream. Without
    /// this case, a helper that bailed out on first-found-match or
    /// only checked singleton `depends_on` lists would pass the
    /// singleton test above and fail here.
    #[test]
    fn referencing_downstreams_includes_a_pipeline_with_a_mixed_depends_on() {
        let others = vec![
            (
                "pl-down-1".to_owned(),
                vec!["pl-up-a".to_owned(), "pl-up-b".to_owned()],
            ),
            ("pl-down-2".to_owned(), vec!["pl-up-b".to_owned()]),
        ];
        assert_eq!(
            referencing_downstreams(&others, "pl-up-b"),
            vec!["pl-down-1".to_owned(), "pl-down-2".to_owned()],
            "both pipelines list pl-up-b among their upstreams"
        );
    }

    /// A person holding `pipeline:read` must not read every tenant's
    /// definitions; the refusal comes before the store is touched.
    #[tokio::test]
    async fn runnable_refuses_a_person_even_with_pipeline_read() {
        let err = runnable(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN);
    }

    /// A service identity passes the check and reaches the store (here
    /// absent, so 503), proving the gate is on the principal kind.
    #[tokio::test]
    async fn runnable_lets_a_service_identity_through_to_the_store() {
        let err = runnable(
            State(state_without_pool()),
            Extension(principal(PrincipalId::Service(Uuid::from_u128(2)))),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    /// A Dagster job is defined in code: editing or deleting it here is a
    /// 404, not a silent no-op.
    #[tokio::test]
    async fn a_dagster_job_cannot_be_edited_or_deleted_here() {
        let body = Bytes::from_static(br#"{"kind":"batch","sourceZone":"a","sourceTable":"b","targetZone":"c","targetTable":"d","schedule":"manual"}"#);
        let err = update(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("gold_export_job".to_owned()),
            body,
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

        let response = delete(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("gold_export_job".to_owned()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_edit_with_a_transform_outside_the_grammar_is_refused() {
        let body = Bytes::from_static(br#"{"kind":"batch","sourceZone":"a","sourceTable":"b","targetZone":"c","targetTable":"d","schedule":"manual","transforms":["filter(1=1; DROP TABLE x)"]}"#);
        let err = update(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("pl-x-1".to_owned()),
            body,
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);
    }

    /// Final-review should-fix (Part 2b): `restore_version` replays a
    /// stored snapshot through `restore_pipeline` without first
    /// validating the snapshot's `depends_on`. A snapshot outlives
    /// one of its declared upstreams — the snapshot was captured
    /// when `pl-b` existed, `pl-b` has since been removed from the
    /// authored graph — and the restore would persist a dangling
    /// reference, the same failure class the delete guard
    /// (`referencing_downstreams`, same file) already refuses to
    /// create on the live graph. The fix wires the same
    /// `validate_depends_on` call `update` runs BEFORE the write,
    /// so the failure case here matches the failure case `update`
    /// returns: a 400 whose message names the dangling id verbatim.
    ///
    /// The route is exercised against a real Postgres so the
    /// "stored snapshot outlives its upstream" precondition can be
    /// built (a stored version row whose `depends_on` references an
    /// id that has been removed from `pipeline_definition`); the
    /// existing `state_without_pool()` fixture returns 503 from
    /// `pool()` before any guard runs, so the validator cannot be
    /// observed at that seam.
    mod restore_version_validates_a_stored_snapshot {
        // Force-link the Postgres testcontainer bootstrap (every
        // other `#[sqlx::test]` in this crate carries the same line).
        use lakehouse_store::pipelines::{self, CreatePipelineInput};
        use lakehouse_test_support as _;

        use super::*;

        /// `DATABASE_URL` dialing the SAME per-test Postgres
        /// `#[sqlx::test]` already handed us via `pool` — extracting
        /// host/port/user/database from the pool's own connect
        /// options, the same shape every other route-level
        /// `sqlx::test` in this crate uses.
        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database")
            )
        }

        /// `AppState` pointed at the test pool. `DAGSTER_URL` is left
        /// at `Config`'s default (`http://localhost:13030/graphql`,
        /// nothing listening in this test environment), so
        /// `state.dagster.list_jobs()` fails and the route falls back
        /// to "no Dagster upstreams accepted" — the same fallback
        /// `update` already exercises. `pl-b` is an authored id and
        /// was never in the Dagster list, so the rejection still
        /// fires on rule 2 ("unknown pipeline").
        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        fn create_input(name: &str, depends_on: Vec<String>) -> CreatePipelineInput {
            CreatePipelineInput {
                name: name.to_owned(),
                kind: "batch".to_owned(),
                source_zone: "bronze".to_owned(),
                source_table: "src".to_owned(),
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: "silver".to_owned(),
                target_table: "tgt".to_owned(),
                schedule: "manual".to_owned(),
                owner: None,
                description: None,
                max_retries: None,
                tenant_id: None,
                depends_on,
                connector_id: None,
            }
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn restore_rejects_a_snapshot_referencing_a_now_deleted_upstream(pool: sqlx::PgPool) {
            // 1. `pl-b` exists — it has to, when its stored snapshot
            //    in `pl-a`'s v=1 row was captured (the store accepts
            //    any `depends_on` value without validating it; the
            //    validator lives at the route layer). The store
            //    slugifies the name with a timestamp suffix, so we
            //    read the id back from the create result rather than
            //    hand-rolling the slug.
            let pl_b = pipelines::create_pipeline(&pool, &create_input("pl-b", Vec::new()), None)
                .await
                .expect("create pl-b");

            // 2. `pl-a` is created with `depends_on = ["pl-b"]`. The
            //    store records this verbatim in v=1's snapshot,
            //    which is exactly the payload `restore_version`
            //    later replays.
            let pl_a = pipelines::create_pipeline(
                &pool,
                &create_input("pl-a", vec![pl_b.id.clone()]),
                None,
            )
            .await
            .expect("create pl-a");

            // 3. `pl-b` is removed from the live graph through the
            //    store directly — the route's `delete` guard would
            //    refuse here because `pl-a` still references it,
            //    which is the same case we are testing against the
            //    restore path. Going through the store models the
            //    "upstream vanished out from under a stored snapshot"
            //    precondition this fix targets.
            let deleted = pipelines::delete_pipeline(&pool, &pl_b.id, None)
                .await
                .expect("delete pl-b");
            assert!(deleted, "pl-b should have been deleted");

            // 4. Restore `pl-a`'s v=1 — its stored snapshot still
            //    lists `pl-b`, which no longer exists. Without the
            //    validator the route happily writes a dangling
            //    reference; with the validator it returns the same
            //    400 `update` returns for a fresh edit against the
            //    same graph, naming `pl-b` verbatim.
            let err = restore_version(
                State(state_for(&pool)),
                Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
                Path((pl_a.id.clone(), 1)),
            )
            .await
            .expect_err("a snapshot whose upstream has been deleted must be refused");
            let response = err.into_response();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "the restore of a stale snapshot must be a 400, not a successful write"
            );
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("collect body");
            let message = String::from_utf8_lossy(&body);
            assert!(
                message.contains(&pl_b.id),
                "the 400 must name the dangling id verbatim: {message:?}"
            );
        }
    }
}
