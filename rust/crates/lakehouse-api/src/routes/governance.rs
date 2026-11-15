//! `GET /api/governance/{kind}`, `GET /api/governance/lineage` — quality,
//! audit, classification, residency, and dataset lineage.
//!
//! Ports `src/app/api/governance/[kind]/route.ts` and
//! `src/app/api/governance/lineage/route.ts`. `lineage` is mounted as a
//! dedicated static route (see `routes/mod.rs`) rather than folded into the
//! `{kind}` dispatch, matching Next.js's separate `lineage/route.ts` file —
//! it is never reached by [`Kind::parse`].

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use lakehouse_store::PgPool;
use lakehouse_store::governance::{
    self, ClassificationRule, CreateClassificationRuleInput, CreatePolicyInput,
    CreateQualityRuleInput, CreateResidencyRuleInput, Policy, QualityRule, ResidencyRule,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashMap;

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::catalog::NOT_IN_PRIMARY;
use crate::routes::lakehouse::{is_unknown_table_error, is_unknown_table_or_database_error};
use crate::routes::support::{nullable_u64_col, str_col};
use crate::state::AppState;
use crate::tenant::{TENANT_ID, TENANT_SITE};
use lakehouse_dagster::{DgClient, DgError, iso_from_unix_seconds, map_run_status};

/// The four recognized `governance/{kind}` values. Ported from the `if
/// (kind === ...)` chain in `governance/[kind]/route.ts`; anything else is
/// [`Kind::Unknown`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `governance/quality` — latest quality-gate verdicts.
    Quality,
    /// `governance/audit` — recent `Dagster` runs as audit events.
    Audit,
    /// `governance/classification` — per-asset classification: the authored
    /// rules, plus a registry row only when a dataset's `tier` is one of the
    /// product's levels (`WEB-3` S3a-D4; the seeded tiers are not).
    Classification,
    /// `governance/residency` — static tenant residency policy.
    ///
    /// # Console surface removed
    ///
    /// `WS1` task 1.12 removed the Residency page: no query path, engine, or
    /// policy check anywhere in the workspace actually enforces a residency
    /// rule, so the page presented authored rows as if they were applied
    /// policy. This route still serves real `Postgres` rows (authored
    /// residency rules, unioned with one hardcoded row — see [`residency`])
    /// and stays registered and `POLICY_TABLE`-classified, kept for `WS7`'s
    /// policy engine to enforce.
    Residency,
    /// `governance/maintenance` — **P4 addition, not a TS-port kind** (like
    /// the `quality`/`classification`/`residency` "Gap fix" unions above,
    /// this extends beyond the original four-kind TS dispatch). Surfaces
    /// `lake.bronze_meta.maintenance_run` — the dry-run/applied
    /// `expire_snapshots` metrics `dagster/dispar_orchestrate/
    /// maintenance.py` writes via the SAME `bronze_meta.*` registry
    /// mechanism `register_bronze_table` already uses, per the task
    /// brief's "reuse that mechanism; do not invent a parallel one."
    Maintenance,
    /// `governance/replication` — **P5 addition, not a TS-port kind**, same
    /// shape as `maintenance`'s "Gap fix" precedent. Surfaces
    /// `lake.bronze_meta.replication_slot` — the per-connector Postgres
    /// replication-slot lag/WAL-retention snapshot
    /// `dagster/dispar_orchestrate/replication_metrics.py` writes on a
    /// schedule, via the SAME `bronze_meta.*` registry mechanism
    /// `register_bronze_table`/`record_maintenance_run` already use. R5 in
    /// the risk register ("a stuck or lagging replication slot pins WAL and
    /// fills the customer's production database disk") is the reason this
    /// exists: it is the first-class metrics surface for that risk, reusing
    /// the P4 maintenance surface's mechanism rather than inventing a
    /// parallel one, per R10.
    Replication,
    /// Anything else, which the TypeScript rejects with HTTP 400.
    Unknown,
}

impl Kind {
    fn parse(kind: &str) -> Self {
        match kind {
            "quality" => Self::Quality,
            "audit" => Self::Audit,
            "classification" => Self::Classification,
            "residency" => Self::Residency,
            "maintenance" => Self::Maintenance,
            "replication" => Self::Replication,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum GovError {
    #[error("{0}")]
    ClickHouse(#[from] ChError),
    #[error("{0}")]
    Dagster(#[from] DgError),
    /// A Postgres failure while fetching authored rules to union into the
    /// list. Note this is distinct from "no pool configured" — that case
    /// is *not* an error: an environment without `DATABASE_URL` simply
    /// gets the `ClickHouse`-only view, matching pre-gap-fix behavior
    /// rather than turning every governance list into a 503.
    #[error("{0}")]
    Store(#[from] lakehouse_store::StoreError),
}

/// `GET /api/governance/{kind}`.
///
/// The quality list names rules of every table; it is narrowed by the
/// catalog's tenant rule and the caller's table access, as
/// [`read_kind`] does (`GOV-15`).
pub async fn get(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(kind): Path<String>,
) -> Response {
    read_kind(&state, Some(&principal), &headers, &kind).await
}

/// The `{kind}` dispatch as one person reads it.
///
/// The route is one caller; the assistant's tools are the other, with the
/// person behind the chat and no request headers of their own. A quality list
/// is narrowed by the catalog's tenant rule and by the caller's table access
/// (`GOV-15 (GOV-12 plan, section 4)`), because every row is a rule on a
/// table. The other kinds are not narrowed by this change — classification
/// and maintenance rows do name tables, but the plan's scope is the quality
/// rules (`GOV-15`); narrowing them is a separate item. No principal is a
/// system call and is not narrowed, which
/// [`crate::routes::quality::quality_table_access`] documents.
pub(crate) async fn read_kind(
    state: &AppState,
    principal: Option<&Principal>,
    headers: &HeaderMap,
    kind: &str,
) -> Response {
    let parsed = Kind::parse(kind);
    // The gate runs before `run`, so a caller the catalog refuses causes no
    // `ClickHouse` query at all (`GOV-15` D8).
    let narrowing = if parsed == Kind::Quality {
        match crate::routes::quality::quality_table_access(state, principal, headers).await {
            Ok(access) => Some(access),
            Err(err) => return ApiRejection(err).into_response(),
        }
    } else {
        None
    };
    match parsed {
        Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("unknown kind: {kind}") })),
        )
            .into_response(),
        parsed => match run(state, parsed).await {
            Ok(mut body) => {
                if let Some(access) = narrowing {
                    retain_readable_rules(&access, &mut body);
                }
                (StatusCode::OK, ApiJson(body)).into_response()
            }
            // Every branch shares one outer `catch (e)` returning
            // `{ error: String(e) }` at 503.
            Err(err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": gov_error_message(&err) })),
            )
                .into_response(),
        },
    }
}

/// Drops every quality row whose `asset` the caller's table access hides, in
/// place — the rule [`crate::routes::catalog::retain_visible`] applies to the
/// catalog's own rows (`GOV-15` D6) — and gives the kept observed rows
/// positional ids from the kept order, so an id says nothing about hidden rows
/// (`GOV-15`, review round 1 (R1)).
///
/// `retain_visible` itself cannot be called: it reads a row's `tableKey` or
/// dotted `id`, while a quality row's key is its `asset` (an authored rule's
/// `id` is a UUID, an observed row's `q-<n>`). A row with no `asset` string is
/// hidden from a narrowed caller, as is an `asset` with no schema: nothing
/// proves the grant covers it (fail closed).
fn retain_readable_rules(access: &crate::table_access::TableAccess, body: &mut Value) {
    if access.is_all() {
        return;
    }
    let Some(rows) = body.get_mut("quality").and_then(Value::as_array_mut) else {
        return;
    };
    rows.retain(|row| {
        row.get("asset")
            .and_then(Value::as_str)
            .map(str::trim)
            .is_some_and(|asset| access.allows_key(asset))
    });
    // The kept observed rows take their positional ids from the kept order
    // (`q-0`, `q-1`, …): an id from the list before the filter would tell the
    // caller how many hidden rows sorted before a visible one (`GOV-15`,
    // review round 1 (R1)). An authored rule's id is a UUID and is left alone.
    for (index, row) in rows
        .iter_mut()
        .filter(|row| {
            row.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| id.starts_with("q-"))
        })
        .enumerate()
    {
        row["id"] = json!(format!("q-{index}"));
    }
}

/// A fixed, classified message for a failed governance read. The raw
/// upstream text used to be returned as `"Error: …"`: the Data Quality
/// page and the copilot showed `DB::Exception: Database _silver_meta does
/// not exist … (version …)` to users (`AGENTS.md` principle 4). A missing
/// database or table is the normal state before a quality or maintenance
/// job has ever written results, so it is named as that.
fn gov_error_message(err: &GovError) -> String {
    match err {
        GovError::ClickHouse(ch) => ch_error_message(ch),
        GovError::Dagster(_) => "Dagster could not be reached to read run history".to_owned(),
        GovError::Store(_) => "database error".to_owned(),
    }
}

/// [`gov_error_message`]'s `ClickHouse` half, shared with
/// [`ingest_runs`].
fn ch_error_message(err: &ChError) -> String {
    let text = err.to_string();
    if text.contains("UNKNOWN_DATABASE") || text.contains("UNKNOWN_TABLE") {
        "no results yet: the job that records them has not run in this deployment".to_owned()
    } else {
        "ClickHouse could not answer this governance query".to_owned()
    }
}

async fn run(state: &AppState, kind: Kind) -> Result<Value, GovError> {
    match kind {
        Kind::Quality => quality(&state.clickhouse, state.pg.as_deref()).await,
        Kind::Audit => audit(&state.dagster, state.pg.as_deref()).await,
        Kind::Classification => classification(&state.clickhouse, state.pg.as_deref()).await,
        Kind::Residency => residency(state.pg.as_deref()).await,
        Kind::Maintenance => maintenance(&state.clickhouse).await,
        Kind::Replication => replication(&state.clickhouse).await,
        Kind::Unknown => unreachable!("Kind::Unknown is handled before `run` is called"),
    }
}

/// `cek === "fail" ? "failed" : cek === "warn" ? "warning" : "passed"`,
/// applied to the *verdict* column (named `v` here to avoid shadowing the
/// `cek` check-name column used by [`severity_of`]).
fn status_of(verdict: &str) -> &'static str {
    match verdict {
        "fail" => "failed",
        "warn" => "warning",
        _ => "passed",
    }
}

/// `verdict === "fail" ? "high" : verdict === "warn" ? "medium" : "info"`.
fn severity_of(verdict: &str) -> &'static str {
    match verdict {
        "fail" => "high",
        "warn" => "medium",
        _ => "info",
    }
}

/// **Gap fix.** `GET /api/governance/quality` — before this fix, a rule
/// authored via `POST /api/governance/quality` (`create_quality_rule`)
/// was written to Postgres but this handler only ever read `ClickHouse`
/// observations, so an authored rule silently never appeared: a 201 that
/// led nowhere. Now the authored rows (via
/// [`lakehouse_store::governance::list_quality_rules`]) are unioned onto
/// the `ClickHouse`-derived list — appended after it, so the
/// `ClickHouse`-observed rows (which is what the UI showed before) keep
/// their existing order and an authored-but-unevaluated rule reads as
/// "newly added", not as displacing real signal. See the module doc
/// comment on `lakehouse_store::governance` for the dedup rule and how an
/// unevaluated rule is represented. When `state.pg` is `None` (no
/// `DATABASE_URL`), this degrades to the pre-fix `ClickHouse`-only view
/// rather than erroring — an authored rule simply cannot exist in that
/// configuration.
async fn quality(ch: &ChClient, pg: Option<&PgPool>) -> Result<Value, GovError> {
    let mut quality = observed_quality(ch, None).await?;
    if let Some(pg) = pg {
        let authored = governance::list_quality_rules(pg).await?;
        // An authored rule carries whether it can be run and what its
        // latest run found (`routes::quality`); an observed one above
        // already carries the verdict its job recorded.
        let runs = crate::routes::quality::latest_runs(ch).await;
        quality.extend(authored.iter().filter_map(|r| {
            let mut rule = serde_json::to_value(r).ok()?;
            crate::routes::quality::annotate_rule(&mut rule, &runs);
            Some(rule)
        }));
    }
    Ok(json!({ "quality": quality }))
}

/// True when `body` is `ClickHouse` saying the table, or its whole
/// database, does not exist. `_silver_meta` is created by whatever job
/// first records a quality verdict, so a deployment where none has run
/// has no database at all (`Code: 81 ... (UNKNOWN_DATABASE)`), not just no
/// table.
fn is_missing_quality_source(body: &str) -> bool {
    is_unknown_table_or_database_error(body) || body.contains("Code: 81.")
}

/// One observed check as a `QualityRule` (`contracts/governance.ts`): the
/// latest verdict a quality job recorded for `(tabel, cek)`.
fn observed_quality_json(index: usize, row: &Map<String, Value>) -> Value {
    let cek = str_col(row, "cek");
    let verdict = str_col(row, "verdict");
    let (name, dimension) = if let Some(col) = cek.strip_prefix("null_rate:") {
        (format!("Column conversion {col}"), "validity")
    } else if cek == "row_count" {
        (cek.to_owned(), "completeness")
    } else {
        (cek.to_owned(), "accuracy")
    };
    let threshold = if cek.starts_with("null_rate") {
        "null <5%"
    } else {
        "row_count > 0 & does not drop >50%"
    };
    json!({
        "id": format!("q-{index}"),
        "name": name,
        "asset": str_col(row, "tabel"),
        "dimension": dimension,
        "threshold": threshold,
        "severity": severity_of(verdict),
        "lastStatus": status_of(verdict),
        "lastRunAt": str_col(row, "at"),
    })
}

/// The latest observed verdict per `(tabel, cek)` from
/// `_silver_meta.quality` — for every table, or only for `tables` (the
/// catalog detail route asks about one asset) — with "nothing has recorded
/// a verdict yet" as the empty list it is. Before this, a missing
/// `_silver_meta` answered 503, so the Data Quality page could not even
/// show the rules people authored. Every other failure still surfaces.
///
/// # Errors
///
/// Returns [`ChError`] for any failure other than a missing table or
/// database.
pub(crate) async fn observed_quality(
    ch: &ChClient,
    tables: Option<&[String]>,
) -> Result<Vec<Value>, ChError> {
    let only = tables.map_or_else(String::new, |tables| {
        let names: Vec<String> = tables
            .iter()
            .map(|t| SqlLiteral::from(t.as_str()).to_string())
            .collect();
        format!("WHERE tabel IN ({}) ", names.join(", "))
    });
    let sql = format!(
        "SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict,
                toString(argMax(nilai, dibuat_pada)) nilai,
                toString(max(dibuat_pada)) at
         FROM _silver_meta.quality {only}GROUP BY tabel, cek ORDER BY tabel, cek LIMIT 500"
    );
    match ch.rows(&sql, None).await {
        Ok(rows) => Ok(rows
            .iter()
            .enumerate()
            .map(|(i, r)| observed_quality_json(i, r))
            .collect()),
        Err(ChError::Server(ref body)) if is_missing_quality_source(body) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

/// `principal_kind`/`outcome` on `audit_event` are a wider vocabulary than
/// the TS `ActorKind` (`"user" | "service" | "agent"`) this response's
/// `actorKind` field is typed as. Map the copilot's `"copilot"` and
/// `"schedule"` principal kinds onto `"agent"` — the closest existing
/// meaning — rather than emit a value the frontend type doesn't know,
/// preserving the existing response shape for existing consumers.
///
/// `None` is a service, not an agent (`SEC-70`, decision D4): an event with
/// no recorded principal is one nobody signed in caused — a refused sign-in,
/// a locked public link, a refused single sign-on link — and the platform
/// recorded it. The actor's own name comes from `actor_label`
/// ([`actor_of`]).
fn actor_kind_of(principal_kind: Option<&str>) -> &'static str {
    match principal_kind {
        Some("user") => "user",
        // "copilot" and "schedule" are both non-human actors from the
        // frontend's point of view.
        Some("copilot" | "schedule") => "agent",
        // A recorded service, and no recorded kind at all (`SEC-70` D4).
        _ => "service",
    }
}

/// The `source` an `audit_event` row reports (QA LH-DATA-15-03 F3, plan D9):
/// where the action came from, following `principal_kind`. No recorded kind
/// is the system, never the assistant (`SEC-70`, decision D4): the event
/// happened before anybody signed in, so nobody else can have sent it.
fn source_of(principal_kind: Option<&str>) -> &'static str {
    match principal_kind {
        Some("user") => "console",
        Some("copilot") => "copilot",
        Some("schedule") => "schedule",
        Some("service") => "service",
        _ => "system",
    }
}

/// The name for an event nobody recorded an actor for. `AI-23` keeps one name
/// for the assistant, but only for an event whose kind *is* `copilot`.
const UNKNOWN_ACTOR: &str = "System";

/// The `actor` an `audit_event` row reports, in order of what the row knows
/// (`SEC-70`, decision D4): the stored label, then the principal id, then —
/// only for an event whose kind is `copilot` and that carries no name —
/// `AI-23`'s one name for the assistant, and otherwise [`UNKNOWN_ACTOR`],
/// which is what rows stored before `SEC-70` read as.
fn actor_of(
    label: Option<&str>,
    principal_id: Option<&str>,
    principal_kind: Option<&str>,
) -> String {
    if let Some(label) = label {
        return label.to_owned();
    }
    if let Some(id) = principal_id {
        return id.to_owned();
    }
    if principal_kind == Some("copilot") {
        return lakehouse_store::agents::COPILOT_EMPLOYEE_NAME.to_owned();
    }
    UNKNOWN_ACTOR.to_owned()
}

/// One sentence saying what a catalog grant or revoke did, for the audit
/// row's `detail` (QA LH-DATA-15-03 F1, plan D9). Built by
/// `catalog_governance::grant_summary` from the stored `permission`,
/// `grantee` and `expiresAt` only, so an unknown `args` blob is never
/// echoed; every other action has no detail.
fn detail_of(action: &str, args: &Value) -> Option<String> {
    if action != "catalog.grant" && action != "catalog.revoke" {
        return None;
    }
    let arg = |key: &str| args.get(key).and_then(Value::as_str);
    Some(super::catalog_governance::grant_summary(action, arg))
}

/// Same narrowing for `outcome`: `audit_event.outcome` has eight values,
/// the TS `AuditOutcome` union has four (`"success" | "denied" | "error" |
/// "pending"`). `needs_confirmation`/`needs_approval` are in-flight: the
/// decision is still open, so they map to `"pending"` and not to
/// `"denied"` — the policy refused nothing — and not to `"success"`,
/// because a viewer scanning for problems must not read a still-pending
/// gate as clean (`GOV-5`).
fn outcome_of(outcome: &str) -> &'static str {
    match outcome {
        "allowed" | "executed" | "approved" => "success",
        "failed" => "error",
        // A decision still waiting on a person: open, not refused.
        "needs_confirmation" | "needs_approval" => "pending",
        // "refused" | "rejected"
        _ => "denied",
    }
}

/// The policy decision a row reports beside its outcome. A row whose
/// decision is still open reports `"pending"` too: the policy allowed
/// nothing and denied nothing, so neither `"allow"` nor `"deny"` would be
/// true (`GOV-5`). A success row stays `"allow"`, everything else stays
/// `"deny"`.
fn policy_decision_of(outcome: &str) -> &'static str {
    match outcome_of(outcome) {
        "success" => "allow",
        "pending" => "pending",
        _ => "deny",
    }
}

/// `GET /api/governance/audit`. Unions `audit_event` rows (copilot/console
/// actions) onto the `Dagster`-run-derived pipeline history, adding a
/// `source` field (`"console" | "copilot" | "schedule" | "service" |
/// "system"` for stored events, `"pipeline"` for runs) so a consumer can
/// filter by origin. `"system"` is an event nobody signed in caused
/// (`SEC-70` D4); no source is ever the assistant unless the event's kind
/// is `copilot`. This is additive: every field the pre-existing response
/// shape
/// had is still present with the same meaning, so
/// `src/services/clients/governance.ts` and the `/audit` page keep
/// working unchanged (see `AuditEvent` in
/// `src/services/contracts/governance.ts` — it declares no `source`
/// field, so TypeScript simply ignores the extra JSON property; nothing
/// currently narrows on it). When `pg` is `None` (no `DATABASE_URL`), this
/// degrades to the pre-fix `Dagster`-only view, matching `quality`'s and
/// `classification`'s same-shaped gap fixes.
async fn audit(dagster: &DgClient, pg: Option<&PgPool>) -> Result<Value, GovError> {
    // Dagster is optional here. It used to be required, so with no
    // orchestrator reachable — which is every local stack, since compose
    // has no Dagster service — the whole audit trail answered 503 even
    // though the console's own events live in Postgres and were right
    // there. Losing the pipeline half of the trail is a gap; losing all
    // of it is a broken page.
    let runs = match dagster.list_runs(50).await {
        Ok(runs) => runs,
        Err(err) => {
            tracing::warn!(%err, "audit: no pipeline history (Dagster unreachable)");
            Vec::new()
        }
    };
    let mut audit: Vec<Value> = runs
        .iter()
        .map(|r| {
            json!({
                "id": r.run_id,
                "at": r.start_time.map_or_else(String::new, iso_from_unix_seconds),
                "actor": "Dagster",
                "actorKind": "service",
                "tenant": TENANT_ID.as_str(),
                "action": format!("pipeline {}: {}", map_run_status(&r.status), r.job_name),
                "resource": r.job_name,
                "outcome": if r.status == "FAILURE" { "error" } else { "success" },
                "policyDecision": "allow",
                "obligations": [],
                "engineCategory": "hot-store",
                "source": "pipeline",
            })
        })
        .collect();
    if let Some(pg) = pg {
        let events =
            lakehouse_store::audit::list(pg, lakehouse_store::audit::AuditFilter::default())
                .await?;
        audit.extend(events.iter().map(|e| {
            let resource = match (&e.resource_kind, &e.resource_id) {
                (Some(kind), Some(id)) => format!("{kind}:{id}"),
                (Some(kind), None) => kind.clone(),
                (None, Some(id)) => id.clone(),
                (None, None) => e.action.clone(),
            };
            let mut row = json!({
                "id": e.id,
                "at": e.at,
                // AI-23 (product owner 2026-11-02): one name for the
                // assistant — "AI Copilot", for an event whose kind is
                // `copilot`; SEC-70 D4: an event nobody signed in caused
                // carries only its label, and reads as "System" otherwise.
                "actor": actor_of(
                    e.actor_label.as_deref(),
                    e.principal_id.as_deref(),
                    e.principal_kind.as_deref(),
                ),
                "actorKind": actor_kind_of(e.principal_kind.as_deref()),
                "tenant": TENANT_ID.as_str(),
                "action": e.action,
                "resource": resource,
                "outcome": outcome_of(&e.outcome),
                "policyDecision": policy_decision_of(&e.outcome),
                "obligations": [],
                "approvalId": e.approval_id,
                // QA LH-DATA-15-03 F3 (plan D9): every row used to say
                // "copilot", whoever acted.
                "source": source_of(e.principal_kind.as_deref()),
            });
            // QA LH-DATA-15-03 F1 (plan D9): the row names who got which
            // permission, from the stored fields only.
            if let (Some(detail), Some(o)) = (detail_of(&e.action, &e.args), row.as_object_mut()) {
                o.insert("detail".to_owned(), json!(detail));
            }
            row
        }));
    }
    // Newest first across both sources: both `at` values are the same
    // fixed-width `YYYY-MM-DDTHH:MM:SS.mmmZ` format, so lexicographic
    // string order is chronological order.
    audit.sort_by(|a, b| {
        let a_at = a.get("at").and_then(Value::as_str).unwrap_or_default();
        let b_at = b.get("at").and_then(Value::as_str).unwrap_or_default();
        b_at.cmp(a_at)
    });
    Ok(json!({ "audit": audit }))
}

/// The product's classification levels: the four the console's badge shows
/// (`src/lib/status.ts` `CLASSIFICATION_LABEL`) and the four
/// `classification_rule`'s `CHECK` accepts (`0003_governance.sql`). A
/// registry `tier` is a SOURCE tier, not a level: `register_bronze_table`
/// writes `primer` and the secondary registry is `sekunder`
/// (`dagster/dispar_orchestrate/bronze_catalog.py`), so no seeded dataset
/// produces a derived classification row — see [`registry_classification`]
/// (`WEB-3` S3a-D4).
const CLASSIFICATION_LEVELS: [&str; 4] = ["public", "internal", "confidential", "restricted"];

/// One classification row derived from a registry dataset, or `None` when
/// its `tier` is not one of [`CLASSIFICATION_LEVELS`] (`WEB-3` S3a-D4).
///
/// The row's `asset` is the key the stored classification rules use for that
/// dataset — `lineageKey` in `src/features/catalog/asset-lineage.tsx`: a
/// Bronze table is `bronze.<table_name>` while the registry has a table name
/// for it, and the slug otherwise. A registry row with a real level as its
/// tier is then matched by the catalog like an authored rule; `primer` and
/// `sekunder` are not levels and produce no row at all, rather than the
/// hardcoded `"internal"` this list used to show for every dataset.
///
/// Pure, so the rule is testable without `ClickHouse`; `title` is not part of
/// the `ClassificationRule` contract, so it is not carried into the row.
fn registry_classification(slug: &str, table_name: &str, tier: &str) -> Option<Value> {
    if !CLASSIFICATION_LEVELS.contains(&tier) {
        return None;
    }
    let asset = if table_name.is_empty() {
        slug.to_owned()
    } else {
        format!("bronze.{table_name}")
    };
    Some(json!({
        "id": format!("c-{slug}"),
        "asset": asset,
        "classification": tier,
        "confidence": 1,
        "reviewStatus": "auto",
    }))
}

/// **Gap fix** — same shape as [`quality`], unioning authored classification
/// rules onto the `ClickHouse`-derived per-asset list.
///
/// DATA-42 D1/D3: the primary registry wins, and every registry table this
/// reader touches is read with `FINAL`.
///
/// D1: the secondary branch drops the slugs the primary registry holds
/// (see [`crate::routes::catalog::NOT_IN_PRIMARY`]), so a slug that is in
/// both registries is classified ONCE — the list used to carry two rows
/// with the same id `c-<slug>`, one per registry.
///
/// D3: `FINAL`, for the reason the catalog's reads give (`DATA-40` D4):
/// the registry is a `ReplacingMergeTree ORDER BY slug` that every run of
/// a connector writes again, and `ClickHouse` merges the copies only in
/// the background. Without it a slug a second run registered was
/// classified twice.
///
/// `WEB-3` S3a-D4: a registry row is reported only when its `tier` is one of
/// the product's classification levels (see [`registry_classification`]).
/// It used to report `"internal"` for every dataset, a level nothing had
/// assigned; the tiers this deployment writes (`primer`, `sekunder`) are not
/// levels, so no registry row appears.
///
/// The `LIMIT 500` stays where it is — after the last `SELECT` of the
/// union — so it caps what it capped before (see the plan's D5: in
/// `ClickHouse` a `LIMIT` after an unparenthesized `UNION ALL` binds to
/// that last `SELECT`, not to the whole union).
async fn classification(ch: &ChClient, pg: Option<&PgPool>) -> Result<Value, GovError> {
    let sql = format!(
        "SELECT slug, table_name, tier FROM lake.`bronze_meta.dataset_catalog` FINAL
         UNION ALL SELECT slug, table_name, tier FROM lake.`bronze_meta_sec.dataset_catalog` FINAL
         WHERE slug {NOT_IN_PRIMARY} LIMIT 500"
    );
    let rows = ch.rows(&sql, None).await?;
    let mut classifications: Vec<Value> = rows
        .iter()
        .filter_map(|r| {
            registry_classification(
                str_col(r, "slug"),
                str_col(r, "table_name"),
                str_col(r, "tier"),
            )
        })
        .collect();
    if let Some(pg) = pg {
        let authored = governance::list_classification_rules(pg).await?;
        classifications.extend(authored.iter().filter_map(|r| serde_json::to_value(r).ok()));
    }
    Ok(json!({ "classifications": classifications }))
}

/// **Gap fix** — same shape as [`quality`], unioning authored residency
/// rules onto the (previously entirely hardcoded, single-row) residency
/// policy list.
async fn residency(pg: Option<&PgPool>) -> Result<Value, GovError> {
    let mut residency: Vec<Value> = vec![json!({
        "id": "res-dispar-dki",
        "tenant": TENANT_ID.as_str(),
        "classification": "internal",
        "approvedSites": [TENANT_SITE.as_str()],
        "crossSiteAllowed": false,
        "allowedOutput": "on-premise DKI",
        "violations7d": 0,
    })];
    if let Some(pg) = pg {
        let authored = governance::list_residency_rules(pg).await?;
        residency.extend(authored.iter().filter_map(|r| serde_json::to_value(r).ok()));
    }
    Ok(json!({ "residency": residency }))
}

/// `GET /api/governance/maintenance` — P4's `dry_run` metrics surface.
/// Reads `lake.bronze_meta.maintenance_run` directly (that table has no
/// authored/Postgres counterpart — it is Dagster-written only — so there
/// is no `pg` union here, unlike `quality`/`classification`/`residency`).
/// The table itself is created by `dagster/dispar_orchestrate/
/// bronze_catalog.py::record_maintenance_run` (`CREATE TABLE IF NOT
/// EXISTS`, single owner — see that module's doc comment on why it is not
/// mirrored into `demo/clickhouse/04_registry.sql`), so on a deployment
/// where the P4 maintenance job has never run, this table does not exist
/// yet and the query below fails — surfaced as the standard 503 `run`
/// already gives every other kind, not a special case.
async fn maintenance(ch: &ChClient) -> Result<Value, GovError> {
    let runs = latest_maintenance_run(ch, None).await?;
    Ok(json!({ "maintenance": runs }))
}

/// The `table_name` value `bronze_meta.maintenance_run` (and
/// `bronze_meta.maintenance_verb_run`) record for a table a caller
/// addresses as `{ns}/{table}`.
///
/// Every writer records the catalog's own two-part name, never the bare
/// one: the Bronze maintenance job (`dagster/dispar_orchestrate/
/// maintenance.py::_run_maintenance_for_table`, fed `bronze.<table>` by
/// `discover_bronze_tables`), `gold_export.py` (`gold.<mart>`) and
/// `agent_runs.py` (`agent_employee.<id>`). A namespace whose writer were
/// to record a bare name would need this function changed — that is the
/// one place the rule lives, so both readers below agree with the writer
/// (AGENTS.md rule 4). `OPS-9`: before this helper, both readers filtered
/// on the bare name from the URL, so a Bronze table never showed the run
/// the job had recorded for it.
pub(crate) fn maintenance_history_table_name(ns: &str, table: &str) -> String {
    format!("{ns}.{table}")
}

/// Builds the `SELECT` behind [`latest_maintenance_run`]. `table_name`
/// narrows to one table (`WHERE table_name = {SqlLiteral} ... LIMIT 1`);
/// `None` keeps the all-tables query byte-identical to what
/// `GET /api/governance/maintenance` has always run (`ORDER BY run_at DESC
/// LIMIT 500`, no `WHERE`).
///
/// A caller narrowing to ONE table passes the name
/// [`maintenance_history_table_name`] built from the `{ns}`/`{table}` it
/// was asked about — the qualified name the writer recorded (see that
/// function's doc comment for why a bare name matches no row).
fn maintenance_run_query(table_name: Option<&str>) -> String {
    const COLUMNS: &str = "table_name, run_at, \
        toString(dry_run_deleted_data_files) dry_data, \
        toString(dry_run_deleted_manifest_files) dry_manifests, \
        toString(applied_deleted_data_files) applied_data, \
        toString(applied_deleted_manifest_files) applied_manifests, \
        skipped_verbs";
    match table_name {
        Some(name) => format!(
            "SELECT {COLUMNS} FROM lake.`bronze_meta.maintenance_run` \
             WHERE table_name = {} ORDER BY run_at DESC LIMIT 1",
            lakehouse_core::ident::SqlLiteral::from(name)
        ),
        None => format!(
            "SELECT {COLUMNS} FROM lake.`bronze_meta.maintenance_run` \
             ORDER BY run_at DESC LIMIT 500"
        ),
    }
}

/// Shared by `GET /api/governance/maintenance` (all tables) and
/// `GET /api/lakehouse/tables/{ns}/{table}/maintenance` (`lastRun`, one
/// table) — the latter narrows the same query rather than re-deriving it.
/// Returns the raw `ChError` (not `GovError`): the lakehouse route
/// classifies it into a fixed 503 itself (never forwarding `ClickHouse`'s
/// own error text), while this module's own `maintenance` handler still
/// converts it into `GovError` via `?`, unchanged from before this
/// extraction.
///
/// # Errors
/// Returns [`ChError`] if the query fails (including the table not
/// existing yet — see this module's doc comment on `maintenance`).
pub(crate) async fn latest_maintenance_run(
    ch: &ChClient,
    table_name: Option<&str>,
) -> Result<Vec<Value>, ChError> {
    let rows = ch.rows(&maintenance_run_query(table_name), None).await?;
    Ok(rows.iter().map(maintenance_run_row_json).collect())
}

/// Maps one `bronze_meta.maintenance_run` row to the JSON shape
/// `GET /api/governance/maintenance` has always returned per row, and that
/// `GET /api/lakehouse/tables/{ns}/{table}/maintenance` reuses verbatim for
/// its `lastRun` field.
pub(crate) fn maintenance_run_row_json(row: &Map<String, Value>) -> Value {
    json!({
        "tableName": str_col(row, "table_name"),
        "runAt": str_col(row, "run_at"),
        "dryRun": {
            "deletedDataFiles": str_col(row, "dry_data"),
            "deletedManifestFiles": str_col(row, "dry_manifests"),
        },
        "applied": {
            "deletedDataFiles": str_col(row, "applied_data"),
            "deletedManifestFiles": str_col(row, "applied_manifests"),
        },
        "skippedVerbs": str_col(row, "skipped_verbs"),
    })
}

/// `GET /api/governance/replication` — R5's slot-lag/WAL-retention metrics
/// surface. Reads `lake.bronze_meta.replication_slot` directly (Dagster-
/// written only, same posture as `maintenance` — no `pg` union). On a
/// deployment where no CDC connector has ever run, this table does not
/// exist yet: that is "no slots", answered as `{"replicationSlots": []}`
/// with 200, not an outage (`WEB-3 review R1` — a fresh install must not
/// open System status on an error). The decision is `ClickHouse`'s own
/// error code ([`CH_UNKNOWN_TABLE`], via [`ChError::server_code`]), never
/// the error text (principle 4); every other failure — `ClickHouse` down, a
/// permission error, a malformed answer — is still the standard 503.
/// The `SELECT` behind [`replication`], pulled out so the
/// `toString(active)` wrapping is assertable — the bug it fixes is invisible
/// in the response shape (a slot just reads inactive) and only reproduces
/// against a real `ClickHouse`, so a unit test on the query text is the
/// cheapest thing that actually guards it.
const REPLICATION_SLOTS_SQL: &str = "SELECT connector_id, slot_name, checked_at, \
     toString(active) active, \
     toString(wal_retained_bytes) wal_retained_bytes, \
     toString(confirmed_flush_lag_bytes) confirmed_flush_lag_bytes, \
     status \
     FROM lake.`bronze_meta.replication_slot` \
     ORDER BY checked_at DESC LIMIT 500";

/// Read a `ClickHouse` boolean-ish column that has been stringified with
/// `toString`.
///
/// `ClickHouse` has no `Bool` in this schema — `active` is `UInt8`, rendered
/// in JSON as the NUMBER `1`/`0`. [`str_col`] is `Value::as_str`, which is
/// `None` for a number, so reading such a column WITHOUT `toString` in the
/// query silently yields `""` and therefore `false`. That is what made every
/// replication slot show as disconnected. Wrapping in `toString` is the fix;
/// this function is the other half of the contract, and accepts `"true"` as
/// well so a future `Bool` column does not silently regress the same way.
fn ch_bool(row: &serde_json::Map<String, Value>, key: &str) -> bool {
    matches!(str_col(row, key), "1" | "true")
}

/// `ClickHouse` error code `UNKNOWN_TABLE`: the table named in the query
/// does not exist. [`replication`] reads it as "no CDC connector has run
/// yet" (`WEB-3 review R1`).
const CH_UNKNOWN_TABLE: u32 = 60;

async fn replication(ch: &ChClient) -> Result<Value, GovError> {
    // WEB-3 review R1: only the engine's UNKNOWN_TABLE code is "no slots
    // yet"; any other error, including one with no code, stays a 503.
    let rows = match ch.rows(REPLICATION_SLOTS_SQL, None).await {
        Ok(rows) => rows,
        Err(err) if err.server_code() == Some(CH_UNKNOWN_TABLE) => Vec::new(),
        Err(err) => return Err(err.into()),
    };
    let slots: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "connectorId": str_col(r, "connector_id"),
                "slotName": str_col(r, "slot_name"),
                "checkedAt": str_col(r, "checked_at"),
                "active": ch_bool(r, "active"),
                "walRetainedBytes": str_col(r, "wal_retained_bytes"),
                "confirmedFlushLagBytes": str_col(r, "confirmed_flush_lag_bytes"),
                "status": str_col(r, "status"),
            })
        })
        .collect();
    Ok(json!({ "replicationSlots": slots }))
}

/// Query parameters accepted by `GET /api/governance/lineage`.
#[derive(Debug, Deserialize)]
pub struct LineageQuery {
    /// The dataset slug to trace. Absent/empty (`?focus=` unset) returns
    /// the empty lineage graph — HTTP 200, not an error — matching
    /// `gov-lineage-empty.json` in the parity corpus.
    #[serde(default)]
    focus: String,
}

/// `GET /api/governance/lineage?focus=<dataset slug or table>`: the
/// recorded lineage around `focus` (see `routes::lineage`). This route
/// used to answer `supported: false` unconditionally; every edge it draws
/// now names the platform record behind it, and an empty or unknown focus
/// returns an empty graph with a note rather than a guess.
///
/// # Errors
///
/// 404 when `X-Tenant` names a tenant the caller does not belong to; 503
/// when the shared-catalog rule cannot be evaluated.
pub async fn lineage(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<lakehouse_auth::Principal>,
    headers: axum::http::HeaderMap,
    Query(q): Query<LineageQuery>,
) -> crate::error::ApiResult<ApiJson<Value>> {
    let body = crate::routes::lineage::build(&state, &principal, &headers, &q.focus).await?;
    Ok(ApiJson(body))
}

/// `GET /api/governance/lineage/columns?table=&column=&direction=up|down`
/// (`DATA-13`): the columns a column comes from, or the columns (and saved
/// charts, with their dashboards) built from it, following recorded mappings
/// up to ten hops. Same permission and tenant rules as [`lineage`]; charts
/// and dashboards are listed only to a caller with `dashboard:read`.
///
/// # Errors
///
/// 400 for a missing or malformed parameter; 404 when `X-Tenant` names a
/// tenant the caller does not belong to; 503 without a Postgres pool or when
/// the shared-catalog rule cannot be evaluated; 500 on a database failure.
pub(crate) async fn column_lineage(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<lakehouse_auth::Principal>,
    headers: axum::http::HeaderMap,
    Query(q): Query<crate::routes::lineage::columns::ColumnQuery>,
) -> crate::error::ApiResult<ApiJson<Value>> {
    let body = crate::routes::lineage::columns::read(&state, &principal, &headers, &q).await?;
    Ok(ApiJson(body))
}

// ── `GET /api/governance/ingest-runs?connectorId=` (WS3 item 17) ───────
//
// Same shape as `lineage` immediately above: a dedicated route (mounted
// next to `lineage` in `routes/mod.rs`), never a seventh `Kind`. Surfaces
// `lake.bronze_meta.ingest_run`, written by
// `dagster/dispar_orchestrate/bronze_catalog.py::record_ingest_run`, to
// the connector detail page's ingest-runs panel.

/// Query parameters accepted by `GET /api/governance/ingest-runs`.
#[derive(Debug, Deserialize)]
pub struct IngestRunsQuery {
    /// Required: axum's `Query` extractor rejects the request with its own
    /// 400 when this is absent, matching this module's `{kind}` dispatch's
    /// own "malformed input -> 4xx before any `ClickHouse` call" posture —
    /// unlike [`LineageQuery::focus`], which is `#[serde(default)]` and
    /// genuinely optional, listing runs for no connector at all is not a
    /// meaningful request.
    #[serde(rename = "connectorId")]
    connector_id: String,
}

/// One row of `lake.bronze_meta.ingest_run`. Mirrors `IngestRun` in
/// `contracts/connectors.ts`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestRunRow {
    pub connector_id: String,
    pub job: String,
    pub object: String,
    /// `None` means "not measured" (`dlt`'s normalize row count was
    /// genuinely unavailable) — never a fabricated `0` (WS3 plan review
    /// Z9, `bronze_catalog.py`'s `_INGEST_RUN_SCHEMA`: `rows` is
    /// `Nullable(UInt64)`, and `record_ingest_run`'s `rows` parameter is
    /// `int | None`, for exactly this reason). Read via
    /// [`nullable_u64_col`], never [`str_col`] plus a client-side parse
    /// that would silently coerce a genuine `NULL` into `0`.
    pub rows: Option<u64>,
    pub started_at: String,
    pub ended_at: String,
    pub status: String,
    pub error: String,
}

/// How many of a connector's most recent per-table results
/// [`ingest_runs_for_connector`] returns.
const INGEST_RUN_ROWS: u32 = 500;

/// Build `connector_id`'s most recent `bronze_meta.ingest_run` rows, newest
/// first.
///
/// `pub(crate)`: `routes::uploads` reads a file load's recorded outcome from
/// the same table, under the id `upload:<upload id>`, instead of carrying
/// its own copy of this query.
///
/// # Errors
///
/// Returns [`ChError`] on a `ClickHouse` transport or server failure.
pub(crate) async fn ingest_runs_for_connector(
    ch: &ChClient,
    connector_id: &str,
) -> Result<Vec<IngestRunRow>, ChError> {
    // Newest first, bounded: the console groups these under the run each
    // one belongs to, and a connector on an hourly schedule records a row
    // per table every hour. `started_at` is ISO-8601 UTC from one writer
    // (`record_ingest_run`), so ordering the text orders the time.
    let sql = format!(
        "SELECT connector_id, job, object, rows, started_at, ended_at, status, error \
         FROM lake.`bronze_meta.ingest_run` WHERE connector_id = {} \
         ORDER BY started_at DESC LIMIT {INGEST_RUN_ROWS}",
        // WS3 plan review Z10: the workspace-wide literal-escaping helper,
        // used identically to `routes::ops::kill_query_sql`/`routes::catalog`
        // for a caller-supplied value in a hand-`format!`ed `ClickHouse`
        // WHERE clause — `governance.rs` had no ClickHouse-literal helper
        // of its own to grep for before this task.
        SqlLiteral::from(connector_id),
    );
    let rows = ch.rows(&sql, None).await?;
    Ok(rows
        .iter()
        .map(|r| IngestRunRow {
            connector_id: str_col(r, "connector_id").to_owned(),
            job: str_col(r, "job").to_owned(),
            object: str_col(r, "object").to_owned(),
            rows: nullable_u64_col(r, "rows"),
            started_at: str_col(r, "started_at").to_owned(),
            ended_at: str_col(r, "ended_at").to_owned(),
            status: str_col(r, "status").to_owned(),
            error: str_col(r, "error").to_owned(),
        })
        .collect())
}

/// [`ingest_runs_for_connector`], with "no run recorded yet" as the empty
/// list it is. `bronze_meta.ingest_run` is created lazily by
/// `dagster/dispar_orchestrate/bronze_catalog.py::record_ingest_run` on
/// the first recorded run, so `ClickHouse` reporting the table does not
/// exist ([`is_unknown_table_error`]) truthfully means nothing has been
/// recorded for any connector — the same reasoning
/// `routes::lakehouse::maintenance_verb_runs_or_empty` applies to its own
/// lazily created table. Every other failure still surfaces.
///
/// `pub(crate)` for the same reason as [`ingest_runs_for_connector`].
///
/// # Errors
///
/// Returns [`ChError`] for any failure other than an unknown table.
pub(crate) async fn ingest_runs_or_empty(
    ch: &ChClient,
    connector_id: &str,
) -> Result<Vec<IngestRunRow>, ChError> {
    match ingest_runs_for_connector(ch, connector_id).await {
        Err(ChError::Server(ref body)) if is_unknown_table_error(body) => Ok(Vec::new()),
        other => other,
    }
}

/// `GET /api/governance/ingest-runs?connectorId=<id>`.
pub async fn ingest_runs(
    State(state): State<AppState>,
    Query(q): Query<IngestRunsQuery>,
) -> Response {
    match ingest_runs_or_empty(&state.clickhouse, &q.connector_id).await {
        Ok(rows) => (StatusCode::OK, ApiJson(rows)).into_response(),
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": ch_error_message(&err) })),
        )
            .into_response(),
    }
}

// ── Postgres-backed writes (Task 2.3) ───────────────────────────────────
//
// Policies (list + create) and the three `create*Rule` handlers below back
// the methods `src/services/clients/governance.ts` used to delegate to
// `mockGovernanceService`. There is no TypeScript server-side precedent for
// any of them (the mock was purely in-browser), so — like
// `routes::identity` — status codes are chosen to be correct rather than
// faithful: 201 on create, 503 when there is no database pool.

/// Borrow the Postgres pool, or fail with a 503 explaining why there isn't
/// one. Mirrors `routes::identity::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "governance store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

fn parse_body<T: serde::de::DeserializeOwned>(body: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))
}

/// Records that something about a catalog table changed — a rule, a
/// classification, a policy or a freshness target was added for it — so
/// the table's asset page can show it in its Change history
/// (`catalog_governance::change_history`). Keyed by the table as the rule
/// names it, lower-cased; an asset is looked up under every key it goes
/// by. Best-effort, like every audit write: it never fails the change it
/// records.
pub(crate) async fn audit_table_change(
    state: &AppState,
    actor: Option<&Principal>,
    table: &str,
    action: &str,
    args: Value,
) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    let table = table.trim().to_lowercase();
    if table.is_empty() {
        return;
    }
    let _ = lakehouse_store::audit::insert(
        pg,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: actor.map(|p| p.id.uuid().to_string()),
            principal_kind: actor.map(|_| "user".to_owned()),
            actor_label: actor.map(|p| p.display_name.clone()),
            action: action.to_owned(),
            resource_kind: Some("catalog".to_owned()),
            resource_id: Some(table),
            args: Some(args),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
}

/// `POST /api/governance/{kind}` — author a new rule for `kind` (`quality`,
/// `classification`, or `residency`; `audit` has no writer, and anything
/// else is unrecognized).
///
/// Dispatches on the same `{kind}` path segment [`get`] reads from, so
/// `GET`/`POST` on one path stay symmetric with every other multi-method
/// route in this router (`/api/alerts`, `/api/dashboard/specs`, ...).
///
/// # Errors
///
/// 400 for an unrecognized `kind` or a malformed body; 403 for the quality
/// kind when the caller is outside the tenant that owns the shared catalog
/// (PR review round 1 R3); 503/500 as above.
pub async fn create_rule(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let actor = principal.as_ref().map(|Extension(p)| p);
    match Kind::parse(&kind) {
        Kind::Quality => {
            match create_quality_rule(State(state.clone()), actor, &headers, body).await {
                Ok(resp) => {
                    let rule = &resp.1.0;
                    audit_table_change(
                        &state,
                        actor,
                        &rule.asset,
                        "quality.rule_create",
                        json!({ "name": rule.name, "threshold": rule.threshold }),
                    )
                    .await;
                    resp.into_response()
                }
                Err(err) => err.into_response(),
            }
        }
        Kind::Classification => {
            match create_classification_rule(State(state.clone()), body).await {
                Ok(resp) => {
                    let rule = &resp.1.0;
                    audit_table_change(
                        &state,
                        actor,
                        &rule.asset,
                        "catalog.classify",
                        json!({ "column": rule.column, "classification": rule.classification }),
                    )
                    .await;
                    resp.into_response()
                }
                Err(err) => err.into_response(),
            }
        }
        Kind::Residency => match create_residency_rule(State(state), body).await {
            Ok(resp) => resp.into_response(),
            Err(err) => err.into_response(),
        },
        Kind::Audit | Kind::Maintenance | Kind::Replication | Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("unknown kind or not writable: {kind}") })),
        )
            .into_response(),
    }
}

/// One authored policy as the console sees it: the stored row plus the two
/// facts `lakehouse-store`'s [`Policy`] cannot answer on its own — whether
/// the policy engine can enforce anything from it (`SEC-61`) and which
/// table page it belongs to (`WEB-3` S3a-D1).
///
/// `enforceable` is deliberately NOT part of `status`: it says whether the
/// rule CAN be enforced, while `status` says whether it is switched on.
/// Both are real and independent, and a `ready` policy with `enforceable:
/// false` masks nothing and filters no rows — the console shows it as "Not
/// enforced" instead of "Enforced" (plan `SEC-61`, decision D3).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyView {
    /// The stored policy, flattened so the wire shape is unchanged apart
    /// from the added `enforceable` and `assetId` fields (`SEC-61` D2,
    /// `WEB-3` S3a-D1).
    #[serde(flatten)]
    policy: Policy,
    /// True when [`crate::policy_engine::PolicyCondition::parse_opt`]
    /// accepts this row's authored `conditions` — the engine's own test,
    /// never a second parser (decision D1). Absent or free-text conditions
    /// and conditions with no real obligation are `false`: a policy with
    /// no structured `conditions` is enforced nowhere.
    enforceable: bool,
    /// The catalog asset id of the table the policy governs, resolved from
    /// `conditions.table` (`WEB-3` S3a-D1). Absent when the conditions do
    /// not parse, name no table the catalog addresses, or name a Bronze
    /// table no registry row describes — the console then shows the table
    /// as plain text rather than linking to an invented page.
    #[serde(skip_serializing_if = "Option::is_none")]
    asset_id: Option<String>,
}

/// Which catalog asset a policy's `conditions.table` names (`WEB-3`
/// S3a-D1). The two zones the catalog addresses differently: a Silver or
/// Serving table is its own asset id (`routes::catalog::detail`), while a
/// Bronze table is known by its registry slug.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PolicyAsset {
    /// `silver.*`/`serving.*` — the table itself is the asset id.
    Table(String),
    /// `bronze.<table>` — the dataset registered under that table name.
    Bronze(String),
}

impl PolicyAsset {
    /// Classifies the table [`crate::policy_engine::PolicyCondition`] parsed
    /// out of a policy's `conditions`. `None` when the conditions do not
    /// parse (legacy prose, absent, or no real obligation), name no zone
    /// and table, or name a zone the catalog does not address this way —
    /// never a guess.
    fn of(conditions: Option<&str>) -> Option<Self> {
        let table = crate::policy_engine::PolicyCondition::parse_opt(conditions)?.table;
        let (zone, name) = table.split_once('.')?;
        match zone {
            "silver" | "serving" => Some(Self::Table(table)),
            "bronze" if !name.is_empty() => Some(Self::Bronze(name.to_owned())),
            _ => None,
        }
    }
}

impl PolicyView {
    /// Wraps `policy` with the engine's verdict on its `conditions` and the
    /// asset id resolved by [`Self::asset_ids`].
    fn new(policy: Policy, asset_id: Option<String>) -> Self {
        let enforceable =
            crate::policy_engine::PolicyCondition::parse_opt(policy.conditions.as_deref())
                .is_some();
        Self {
            policy,
            enforceable,
            asset_id,
        }
    }

    /// One asset id per policy, in order: a Silver/Serving table is its own
    /// id, and a Bronze table's id is its registry slug from the same
    /// reader the catalog uses ([`crate::routes::catalog::registered_slug`],
    /// rule 4), read once per distinct table. A registry that cannot answer
    /// costs the link, never the list — the same choice
    /// [`crate::routes::catalog::registered_slug`]'s Bronze upstream makes.
    async fn asset_ids(ch: &ChClient, policies: &[Policy]) -> Vec<Option<String>> {
        let mut bronze: HashMap<String, Option<String>> = HashMap::new();
        let mut ids = Vec::with_capacity(policies.len());
        for policy in policies {
            ids.push(match PolicyAsset::of(policy.conditions.as_deref()) {
                Some(PolicyAsset::Table(table)) => Some(table),
                Some(PolicyAsset::Bronze(table)) => {
                    if let Some(found) = bronze.get(&table) {
                        found.clone()
                    } else {
                        let found = crate::routes::catalog::registered_slug(ch, &table)
                            .await
                            .ok()
                            .flatten();
                        bronze.insert(table, found.clone());
                        found
                    }
                }
                None => None,
            });
        }
        ids
    }
}

/// The body of `GET /api/governance/row-filter-grammar` (`WEB-8` S2 T8,
/// D8; forms added by S2 review round 1, S2): the function names, operator
/// tokens and expression forms [`row_filter_grammar`] serves, read directly
/// from the constants in [`crate::sql_rewrite`] — never a second list, so a
/// change to what a save accepts changes the console's help line with it.
#[derive(Debug, Serialize)]
pub struct RowFilterGrammar {
    /// Function names, in `ALLOWED_ROW_FILTER_FUNCTIONS`'s order.
    functions: &'static [&'static str],
    /// Operator tokens, in `ALLOWED_ROW_FILTER_OPERATORS`'s order.
    operators: &'static [&'static str],
    /// Expression forms, in `ALLOWED_ROW_FILTER_FORMS`'s order.
    forms: &'static [&'static str],
}

/// `GET /api/governance/row-filter-grammar` (`WEB-8` S2 T8, D8; forms added
/// by S2 review round 1, S2) — the functions, operators and forms a row
/// filter may use. Gated by `policy:read` in
/// [`POLICY_TABLE`](crate::policy::POLICY_TABLE): the same grant that lists
/// the policies this grammar authors.
///
/// The answer is built from
/// [`crate::sql_rewrite::ALLOWED_ROW_FILTER_FUNCTIONS`],
/// [`crate::sql_rewrite::ALLOWED_ROW_FILTER_OPERATORS`] and
/// [`crate::sql_rewrite::ALLOWED_ROW_FILTER_FORMS`] (rule 4: no second copy,
/// in Rust or TypeScript). The forms are shapes rather than a membership
/// list; the test below feeds each name back to the validator, which is what
/// keeps this list honest.
///
/// # Errors
///
/// This handler has no failing path; it returns `ApiResult` only so every
/// route in this crate has the same `ApiResult<ApiJson<T>>` shape.
pub async fn row_filter_grammar() -> ApiResult<ApiJson<RowFilterGrammar>> {
    Ok(ApiJson(RowFilterGrammar {
        functions: crate::sql_rewrite::ALLOWED_ROW_FILTER_FUNCTIONS,
        operators: crate::sql_rewrite::ALLOWED_ROW_FILTER_OPERATORS,
        forms: crate::sql_rewrite::ALLOWED_ROW_FILTER_FORMS,
    }))
}

/// `GET /api/governance/policies` — every authored policy, each with the
/// engine's verdict on whether it can be enforced (`SEC-61`) and the catalog
/// asset it governs, where one can be named (`WEB-3` S3a-D1).
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_policies(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<PolicyView>>> {
    let policies = governance::list_policies(pool(&state)?).await?;
    let ids = PolicyView::asset_ids(&state.clickhouse, &policies).await;
    Ok(ApiJson(
        policies
            .into_iter()
            .zip(ids)
            .map(|(policy, asset_id)| PolicyView::new(policy, asset_id))
            .collect(),
    ))
}

/// [`PolicyView::new`] for one policy, with its asset id resolved
/// (`WEB-3` S3a-D1).
async fn policy_view(state: &AppState, policy: Policy) -> PolicyView {
    let asset_id = PolicyView::asset_ids(&state.clickhouse, std::slice::from_ref(&policy))
        .await
        .into_iter()
        .next()
        .flatten();
    PolicyView::new(policy, asset_id)
}

/// The `POST /api/governance/policies` body. Mirrors `CreatePolicyInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePolicyBody {
    name: String,
    kind: String,
    subjects: String,
    resources: String,
    effect: String,
    #[serde(default)]
    conditions: Option<String>,
    #[serde(default)]
    activate: bool,
    #[serde(default)]
    owner: Option<String>,
}

/// Parse and validate a `POST /api/governance/policies` body into a
/// [`CreatePolicyInput`], pure and DB-free so it can be unit tested without
/// a pool (same extraction pattern as `routes::query::run_result_json`).
///
/// Refuses (400) a `conditions` value that parses as JSON but describes no
/// real obligation — [`crate::policy_engine::PolicyCondition::parse`]
/// returns `None` for it. A `conditions` value that is present but is NOT
/// valid JSON at all (legacy free-text prose) passes through unchanged:
/// only a JSON-shaped-but-empty condition is refused, so an admin who
/// deliberately writes prose is never blocked, but one who almost-authors
/// a structured clause is caught before saving something that silently
/// does nothing.
///
/// # Errors
///
/// Returns [`ApiError::BadRequest`] if `body` isn't the expected shape, or
/// if an authored `conditions` blob is JSON-shaped but enforces nothing.
fn create_policy_body(body: Value) -> Result<CreatePolicyInput, ApiError> {
    let body: CreatePolicyBody = serde_json::from_value(body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
    if let Some(conditions) = body.conditions.as_deref() {
        let is_json = serde_json::from_str::<Value>(conditions).is_ok();
        if is_json && crate::policy_engine::PolicyCondition::parse(conditions).is_none() {
            return Err(ApiError::BadRequest(
                "conditions authors an enforcement clause with no obligation (mask or rowFilter required)"
                    .to_owned(),
            ));
        }
    }
    Ok(CreatePolicyInput {
        name: body.name,
        kind: body.kind,
        subjects: body.subjects,
        resources: body.resources,
        effect: body.effect,
        conditions: body.conditions,
        activate: body.activate,
        owner: body.owner,
    })
}

/// `POST /api/governance/policies` — author a new policy. Returns 201, with
/// the same `enforceable` verdict [`list_policies`] reports (decision D2).
///
/// # Errors
///
/// 400 on a malformed body or an authored `conditions` blob with no real
/// obligation; 409 if the name is taken; 503/500 as above.
pub async fn create_policy(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<PolicyView>)> {
    let body: Value = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
    let input = create_policy_body(body)?;
    let created = governance::create_policy(pool(&state)?, &input).await?;
    let view = policy_view(&state, created).await;
    Ok((StatusCode::CREATED, ApiJson(view)))
}

/// `POST /api/governance/policies` as the router mounts it:
/// [`create_policy`], plus a Change-history entry on the table the new
/// policy binds, when its condition names one.
///
/// # Errors
///
/// Exactly [`create_policy`]'s.
pub async fn create_policy_route(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<PolicyView>)> {
    let created = create_policy(State(state.clone()), body).await?;
    let policy = &created.1.0.policy;
    if let Some(cond) =
        crate::policy_engine::PolicyCondition::parse_opt(policy.conditions.as_deref())
    {
        audit_table_change(
            &state,
            principal.as_ref().map(|Extension(p)| p),
            &cond.table,
            "policy.create",
            json!({ "name": policy.name }),
        )
        .await;
    }
    Ok(created)
}

/// Records a change to a policy in the audit trail: on the table its
/// condition binds, where that table's asset page shows it in its Change
/// history ([`audit_table_change`]) — or, for a policy that binds no
/// table, on the policy itself. One event either way.
async fn audit_policy_change(
    state: &AppState,
    actor: Option<&Principal>,
    policy: &Policy,
    action: &str,
    args: Value,
) {
    if let Some(cond) =
        crate::policy_engine::PolicyCondition::parse_opt(policy.conditions.as_deref())
    {
        audit_table_change(state, actor, &cond.table, action, args).await;
        return;
    }
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    let _ = lakehouse_store::audit::insert(
        pg,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: actor.map(|p| p.id.uuid().to_string()),
            principal_kind: actor.map(|_| "user".to_owned()),
            actor_label: actor.map(|p| p.display_name.clone()),
            action: action.to_owned(),
            resource_kind: Some("policy".to_owned()),
            resource_id: Some(policy.id.clone()),
            args: Some(args),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
}

/// The `PUT /api/governance/policies/{id}/status` body.
#[derive(Debug, Deserialize)]
pub struct PolicyStatusBody {
    status: String,
}

/// `PUT /api/governance/policies/{id}/status` — enforce a policy
/// (`"ready"`) or stop enforcing it (`"draft"`). Until this route a policy
/// kept the status it was created with for life: a draft could never be
/// enforced, and an enforced one could never be stopped. It takes effect
/// on the next query — obligations are read from the store each time.
///
/// The response carries `enforceable` too (decision D2): switching a policy
/// on says nothing about whether the engine can enforce it, and the console
/// must keep both facts apart when it updates the row.
///
/// # Errors
///
/// `400` for a malformed body or any other status; `404` when no policy
/// has that id; 503/500 as above.
pub async fn set_policy_status(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<PolicyView>> {
    let body: PolicyStatusBody = parse_body(&body)?;
    if !matches!(body.status.as_str(), "ready" | "draft") {
        return Err(ApiError::BadRequest(
            "status must be \"ready\" (enforced) or \"draft\" (not enforced)".to_owned(),
        )
        .into());
    }
    let pool = pool(&state)?;
    let before = governance::list_policies(pool)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| ApiError::NotFound("Policy not found".to_owned()))?;
    let policy = governance::set_policy_status(pool, &id, &body.status)
        .await?
        .ok_or_else(|| ApiError::NotFound("Policy not found".to_owned()))?;
    if before.status != policy.status {
        let action = if policy.status == "ready" {
            "policy.enforce"
        } else {
            "policy.suspend"
        };
        audit_policy_change(
            &state,
            principal.as_ref().map(|Extension(p)| p),
            &policy,
            action,
            json!({ "name": policy.name }),
        )
        .await;
    }
    let view = policy_view(&state, policy).await;
    Ok(ApiJson(view))
}

/// `DELETE /api/governance/policies/{id}` — remove a policy. An enforced
/// one stops masking and filtering from the next query on, which is why
/// the audit event says whether it was (`enforced`).
///
/// # Errors
///
/// `404` when no policy has that id; 503/500 as above.
pub async fn delete_policy(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let policy = governance::delete_policy(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Policy not found".to_owned()))?;
    audit_policy_change(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        &policy,
        "policy.delete",
        json!({ "name": policy.name, "enforced": policy.status == "ready" }),
    )
    .await;
    Ok(ApiJson(json!({ "ok": true, "id": policy.id })))
}

/// The `POST /api/governance/policies/preview` body — a `table`/`mask`/
/// `rowFilter` triple an admin is drafting, not-yet-saved.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPolicyBody {
    table: String,
    #[serde(default)]
    mask: Vec<String>,
    #[serde(default)]
    row_filter: Option<String>,
}

/// Builds the `POST /api/governance/policies/preview` response body.
/// `affected_rows`/`masked_columns` are the caller's own real
/// measurements — `None` for either means "could not measure", never a
/// fabricated `0`/`[]` claimed as real — and `supported` is `true` only
/// when BOTH measurements actually happened, matching WS7 item A5's Step 1
/// tests: `(None, None)` for an unmeasurable preview (an unparsed row
/// filter, or a table `system.columns` doesn't know), `(Some(n),
/// Some(cols))` for a real one.
///
/// # Errors
/// This function itself never errors — it is a pure `Value` builder, kept
/// separate from the route handler so it is unit-testable without a
/// `ClickHouse` connection (the same extraction pattern
/// `routes::query::run_result_json` already uses).
#[must_use]
fn preview_result_json(affected_rows: Option<i64>, masked_columns: Option<Vec<String>>) -> Value {
    let supported = affected_rows.is_some() && masked_columns.is_some();
    json!({
        "affectedRows": affected_rows,
        "maskedColumns": masked_columns.unwrap_or_default(),
        "supported": supported,
    })
}

/// `POST /api/governance/policies/preview` — real policy impact preview
/// (WS7 item A5, closing WS1 task 12's deferred half:
/// `docs/superpowers/plans/2026-09-10-ws1-honesty-pass.md:912`, "Delete
/// that block. Do not replace it — WS7's policy engine can compute a real
/// one."). Re-validates `rowFilter` through the SAME
/// `sql_rewrite::validate_row_filter_expr` Phase B built — never a second,
/// divergent grammar (this is exactly why A5 was deferred until Phase B
/// existed) — runs `SELECT count() FROM <table> WHERE <rewritten filter>`
/// against `ClickHouse` (bounded 5s timeout) for `affectedRows`, and
/// cross-checks each `mask` entry against `system.columns` for that
/// table, reporting only the ones that are real columns (a typo'd column
/// name is silently dropped from the PREVIEW only — `create_policy`
/// (WS7 item A4) does not cross-check column existence at all, since a table
/// can gain a masked column later, so this asymmetry is disclosed, not a
/// bug).
///
/// Gated by `policy:write` (`POLICY_TABLE`) — only someone who could
/// actually save the policy may preview its effect.
///
/// # Errors
///
/// 400 on a malformed body; 503 if no pool is configured (kept consistent
/// with every other `/api/governance/*` route, even though this route
/// itself never touches Postgres — `pool(&state)?` is the same
/// availability gate the rest of this file uses).
pub async fn preview_policy(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    pool(&state)?;
    let body: PreviewPolicyBody = parse_body(&body)?;
    Ok(ApiJson(preview_policy_impl(&state, &body).await))
}

async fn preview_policy_impl(state: &AppState, body: &PreviewPolicyBody) -> Value {
    let Ok((schema, table)) = lakehouse_core::ident::split_namespaced_table(&body.table) else {
        return preview_result_json(None, None);
    };
    let cols_sql = format!(
        "SELECT name FROM system.columns WHERE database={} AND table={}",
        SqlLiteral::from(schema),
        SqlLiteral::from(table),
    );
    let real_columns: Vec<String> = match state.clickhouse.rows(&cols_sql, None).await {
        Ok(rows) => rows
            .iter()
            .filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_owned))
            .collect(),
        Err(_) => Vec::new(),
    };
    if real_columns.is_empty() {
        // Either the table genuinely has no columns `ClickHouse` reports
        // (does not exist yet, or the read itself failed) — either way
        // this module cannot measure anything real for it.
        return preview_result_json(None, None);
    }
    let masked_columns: Vec<String> = body
        .mask
        .iter()
        .filter(|m| real_columns.iter().any(|c| c.eq_ignore_ascii_case(m)))
        .cloned()
        .collect();

    let where_clause = match body
        .row_filter
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
    {
        Some(filter) => {
            let expanded = crate::sql_rewrite::validate_row_filter_expr(
                filter,
                &real_columns,
                crate::sql_rewrite::Engine::ClickHouse,
            )
            .and_then(|expr| {
                crate::sql_rewrite::expand_placeholders(
                    &expr,
                    &crate::sql_rewrite::PlaceholderValues::none(),
                )
            });
            match expanded {
                Ok(expanded) => format!(" WHERE {expanded}"),
                // Same "conditions do not parse" shape Step 1's own test
                // exercises — `affectedRows` stays unmeasured, but the
                // masked-column cross-check already ran, so it is still
                // reported (never re-nulled just because the OTHER half
                // of the preview failed).
                Err(_) => return preview_result_json(None, Some(masked_columns)),
            }
        }
        None => String::new(),
    };
    let count_sql =
        format!("SELECT toString(count()) AS n FROM `{schema}`.`{table}`{where_clause}");
    let affected_rows = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        state.clickhouse.rows(&count_sql, None),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .and_then(|rows| rows.into_iter().next())
    .and_then(|row| {
        row.get("n")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i64>().ok())
    });

    preview_result_json(affected_rows, Some(masked_columns))
}

/// The `POST /api/governance/quality` body. Mirrors `CreateQualityRuleInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateQualityRuleBody {
    name: String,
    asset: String,
    dimension: String,
    threshold: String,
    severity: String,
}

/// `POST /api/governance/quality` — author a new data-quality rule. Returns
/// 201.
///
/// Distinct from `GET /api/governance/quality` ([`get`] with
/// `Kind::Quality`), which stays `ClickHouse`-backed and unaffected by this
/// handler — see the module doc comment on `lakehouse_store::governance`.
///
/// # Errors
///
/// 400 on a malformed body, or for a table that is not there or that the
/// author's roles do not let them read (QA `LH-DATA-14-01` F2, `DATA-54`);
/// 403 when the caller is outside the tenant that owns the shared catalog
/// (PR review round 1 R3); 409 if the name is taken; 503 when the table
/// cannot be checked; 500 on a store failure.
pub async fn create_quality_rule(
    State(state): State<AppState>,
    actor: Option<&Principal>,
    headers: &HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<QualityRule>)> {
    let body: CreateQualityRuleBody = parse_body(&body)?;
    crate::routes::quality::ensure_rule_table(&state, actor, headers, &body.asset).await?;
    let input = CreateQualityRuleInput {
        name: body.name,
        asset: body.asset,
        dimension: body.dimension,
        threshold: body.threshold,
        severity: body.severity,
    };
    let created = governance::create_quality_rule(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// The `POST /api/governance/classification` body. Mirrors
/// `CreateClassificationRuleInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateClassificationRuleBody {
    asset: String,
    #[serde(default)]
    column: Option<String>,
    classification: String,
    #[serde(default, rename = "maskingRule")]
    masking_rule: Option<String>,
}

/// `POST /api/governance/classification` — author a new classification/
/// masking rule. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body; 503/500 as above.
pub async fn create_classification_rule(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<ClassificationRule>)> {
    let body: CreateClassificationRuleBody = parse_body(&body)?;
    let input = CreateClassificationRuleInput {
        asset: body.asset,
        column: body.column,
        classification: body.classification,
        masking_rule: body.masking_rule,
    };
    let created = governance::create_classification_rule(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

/// The `POST /api/governance/residency` body. Mirrors
/// `CreateResidencyRuleInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateResidencyRuleBody {
    tenant: String,
    classification: String,
    #[serde(default)]
    approved_sites: Vec<String>,
    #[serde(default)]
    cross_site_allowed: bool,
    #[serde(default)]
    allowed_output: String,
}

/// `POST /api/governance/residency` — author a new residency rule. Returns
/// 201.
///
/// # Errors
///
/// 400 on a malformed body; 503/500 as above.
pub async fn create_residency_rule(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<ResidencyRule>)> {
    let body: CreateResidencyRuleBody = parse_body(&body)?;
    let input = CreateResidencyRuleInput {
        tenant: body.tenant,
        classification: body.classification,
        approved_sites: body.approved_sites,
        cross_site_allowed: body.cross_site_allowed,
        allowed_output: body.allowed_output,
    };
    let created = governance::create_residency_rule(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
}

// ── Dataset SLA (WS5 item E1, Y6) ────────────────────────────────────────
//
// Mounted as literal routes ahead of the generic `/api/governance/{kind}`
// fallback (`routes/mod.rs`, `policy.rs`), matching the existing
// `/api/governance/lineage`/`/api/governance/policies` precedent — never a
// seventh `{kind}` dispatch value (see this module's doc comment).

/// Validate `raw` as `<namespace>.<table>`, each half a real
/// [`lakehouse_core::ident::Ident`].
///
/// Wraps [`lakehouse_core::ident::split_namespaced_table`] — the single
/// shared guard both this route and `lakehouse-alerts`'s
/// `normalize_freshness` call (WS5 item C1a review finding: this was
/// written twice, two commits apart, because `lakehouse-alerts` cannot
/// depend on `lakehouse-api` to reuse a copy living here; `lakehouse-core`
/// sits below both). `Ident` only guarantees lexical safety (no SQL
/// injection through the identifier position); `dataset_sla.table_name`
/// is never interpolated into a query, so that guarantee is stronger than
/// this call site strictly needs — but it is also the exact
/// `<namespace>.<table>` shape check the route needs.
fn validate_namespaced_table(raw: &str) -> Result<(), ApiError> {
    lakehouse_core::ident::split_namespaced_table(raw)
        .map(|_| ())
        .map_err(|err| {
            ApiError::BadRequest(match err {
                lakehouse_core::ident::NamespacedTableError::MissingSeparator => {
                    "tableName wajib berformat <namespace>.<table>.".to_owned()
                }
                lakehouse_core::ident::NamespacedTableError::Invalid(_) => {
                    "tableName tidak valid.".to_owned()
                }
            })
        })
}

/// `GET /api/governance/sla` — every authored dataset freshness SLA.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn get_sla(
    State(state): State<AppState>,
) -> ApiResult<ApiJson<Vec<governance::DatasetSla>>> {
    Ok(ApiJson(governance::list_dataset_sla(pool(&state)?).await?))
}

/// The `PUT /api/governance/sla` body. Mirrors `DatasetSla`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PutDatasetSlaBody {
    table_name: String,
    expected_interval_minutes: i32,
    #[serde(default)]
    owner: Option<String>,
}

/// `PUT /api/governance/sla` — author or replace one table's freshness SLA.
/// Gated by `governance:write` (`POLICY_TABLE`), granted to the
/// `Governance Admin` role by `0030_table_maintenance_policy.sql` — not
/// re-granted here.
///
/// # Errors
///
/// 400 if `tableName` is not `<namespace>.<table>` (each half a valid
/// [`lakehouse_core::ident::Ident`]) or `expectedIntervalMinutes` is not
/// `> 0` — the route-level half of WS5 plan review U12's defense in depth;
/// `0037_dataset_sla.sql`'s `CHECK (expected_interval_minutes > 0)` is the
/// guarantee this mirrors, not the other way around. 503/500 as above.
pub async fn put_sla(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<governance::DatasetSla>> {
    let body: PutDatasetSlaBody = parse_body(&body)?;
    validate_namespaced_table(&body.table_name)?;
    if body.expected_interval_minutes <= 0 {
        return Err(ApiError::BadRequest(
            "expectedIntervalMinutes wajib bernilai positif.".to_owned(),
        )
        .into());
    }
    let input = governance::DatasetSla {
        table_name: body.table_name,
        expected_interval_minutes: body.expected_interval_minutes,
        owner: body.owner,
    };
    let saved = governance::upsert_dataset_sla(pool(&state)?, &input).await?;
    audit_table_change(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        &saved.table_name,
        "catalog.sla_set",
        json!({ "minutes": saved.expected_interval_minutes }),
    )
    .await;
    Ok(ApiJson(saved))
}

/// `DELETE /api/governance/sla/{table}` — remove one table's freshness
/// SLA. Until this route a target, once set, could only be changed: a
/// table judged late against a target set by mistake stayed degraded.
/// Gated by `governance:write`, like setting one.
///
/// # Errors
///
/// 404 when the table has no SLA; 503/500 as above.
pub async fn delete_sla(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(table): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let removed = governance::delete_dataset_sla(pool(&state)?, &table)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound("No freshness target is set for this table".to_owned())
        })?;
    audit_table_change(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        &removed.table_name,
        "catalog.sla_remove",
        json!({ "minutes": removed.expected_interval_minutes }),
    )
    .await;
    Ok(ApiJson(
        json!({ "ok": true, "tableName": removed.table_name }),
    ))
}

/// `DELETE /api/governance/classification/{id}` — remove a classification
/// rule. A classification could only be overridden by adding a newer rule,
/// never taken back; with the rule gone, an older rule for the same asset
/// or column applies again, or the default level. Gated by
/// `governance:write`.
///
/// # Errors
///
/// 404 when no rule has that id; 503/500 as above.
pub async fn delete_classification_rule(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let rule = governance::delete_classification_rule(pool(&state)?, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound("Classification rule not found".to_owned()))?;
    audit_table_change(
        &state,
        principal.as_ref().map(|Extension(p)| p),
        &rule.asset,
        "catalog.declassify",
        json!({ "column": rule.column, "classification": rule.classification }),
    )
    .await;
    Ok(ApiJson(json!({ "ok": true, "id": rule.id })))
}

// ── /api/governance/tag-keys (DATA-12 part 2) ──────────────────────────

/// Longest display name stored as `created_by` (`0083`'s CHECK).
const TAG_KEY_ACTOR_MAX_LEN: usize = 128;

fn tag_key_json(row: &lakehouse_store::governed_tag::TagKeyRow) -> Value {
    let iso = |at: time::OffsetDateTime| {
        at.format(&time::format_description::well_known::Rfc3339)
            .ok()
    };
    json!({
        "key": row.key,
        "allowedValues": row.allowed_values,
        "createdBy": row.created_by,
        "createdAt": iso(row.created_at),
        "updatedAt": iso(row.updated_at),
    })
}

/// Records a tag-key change in the audit trail, on the key itself (a key is
/// not a catalog table, so it has no Change history to join). Best-effort
/// like every audit write here: it never fails the change it records.
async fn audit_tag_key(state: &AppState, actor: &Principal, key: &str, action: &str, args: Value) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    let _ = lakehouse_store::audit::insert(
        pg,
        lakehouse_store::audit::NewAuditEvent {
            principal_id: Some(actor.id.uuid().to_string()),
            principal_kind: Some("user".to_owned()),
            actor_label: Some(actor.display_name.clone()),
            action: action.to_owned(),
            resource_kind: Some("tag_key".to_owned()),
            resource_id: Some(key.to_owned()),
            args: Some(args),
            outcome: "executed".to_owned(),
            ..Default::default()
        },
    )
    .await;
}

/// `GET /api/governance/tag-keys` — every defined governed tag key with the
/// values it allows. Needs only `catalog:read` (`POLICY_TABLE`): an editor
/// with `catalog:write` must read the list to render the value pickers, and
/// a definition is not sensitive.
///
/// # Errors
///
/// 503 if no pool is configured; 500 (classified, never database text) on a
/// database failure.
pub async fn list_tag_keys(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<Value>>> {
    // PR #106 review R2 SHOULD-FIX 2: gate list_tag_keys with catalog_tenant_refusal
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let rows = lakehouse_store::governed_tag::list_keys(pool(&state)?).await?;
    Ok(ApiJson(rows.iter().map(tag_key_json).collect()))
}

/// The `POST /api/governance/tag-keys` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateTagKeyBody {
    key: String,
    allowed_values: Vec<String>,
}

/// `POST /api/governance/tag-keys` — define a key and the values it may
/// take. Needs `governance:write`: only an admin creates keys (spec,
/// "Governed tags"). Audited as `governance.tag_key_create`.
///
/// # Errors
///
/// 400 naming the field for a malformed body, a key or value that is not
/// 1-64 characters matching `^[a-z0-9][a-z0-9_-]*$`, an empty or repeating
/// list, or more than 50 values; 409 when the key already exists; 503/500
/// as above.
pub async fn create_tag_key(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 BLOCKER 1
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let body: CreateTagKeyBody = parse_body(&body)?;
    let created_by: String = principal
        .display_name
        .chars()
        .take(TAG_KEY_ACTOR_MAX_LEN)
        .collect();
    let row = lakehouse_store::governed_tag::create_key(
        pool(&state)?,
        &body.key,
        &body.allowed_values,
        &created_by,
    )
    .await
    .map_err(|err| match err {
        lakehouse_store::StoreError::Conflict => {
            ApiError::Conflict(format!("the tag key \"{}\" already exists", body.key))
        }
        other => ApiError::from(other),
    })?;
    audit_tag_key(
        &state,
        &principal,
        &row.key,
        "governance.tag_key_create",
        json!({ "allowedValues": row.allowed_values }),
    )
    .await;
    Ok(ApiJson(tag_key_json(&row)))
}

/// The `PUT /api/governance/tag-keys/{key}` body. Only the values change: a
/// key is never renamed, so a `key` field is refused rather than ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateTagKeyBody {
    allowed_values: Vec<String>,
}

/// `PUT /api/governance/tag-keys/{key}` — replace the values a key allows.
/// Needs `governance:write`. A value some table still has cannot be removed:
/// the refusal names each such value with its table count and nothing
/// changes. Audited as `governance.tag_key_update`.
///
/// # Errors
///
/// 400 for a malformed body (including a `key` field), the value rules of
/// [`create_tag_key`], or a removed value still in use; 404 when the key is
/// not defined; 503/500 as above.
pub async fn update_tag_key(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 BLOCKER 1
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    let body: UpdateTagKeyBody = parse_body(&body)?;
    let row =
        lakehouse_store::governed_tag::update_key_values(pool(&state)?, &key, &body.allowed_values)
            .await
            .map_err(|err| match err {
                lakehouse_store::StoreError::NotFound => tag_key_not_found(&key),
                other => ApiError::from(other),
            })?;
    audit_tag_key(
        &state,
        &principal,
        &row.key,
        "governance.tag_key_update",
        json!({ "allowedValues": row.allowed_values }),
    )
    .await;
    Ok(ApiJson(tag_key_json(&row)))
}

/// `DELETE /api/governance/tag-keys/{key}` — remove a key nobody uses.
/// Needs `governance:write`. Refused while any table has a value for it: a
/// tag is never lost silently. The sentence counts the tables and names up to
/// three of them — only the ones the caller may read, the rest are counted
/// (QA `LH-DATA-12-15`, `DATA-54`). Audited as
/// `governance.tag_key_delete`.
///
/// # Errors
///
/// 400 when a table still has a value for the key; 404 when the key is not
/// defined; 503/500 as above.
pub async fn delete_tag_key(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    // PR #106 review R1 BLOCKER 1
    if let Some(reason) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await?
    {
        return Err(ApiError::PermissionDenied(reason.to_owned()).into());
    }
    match lakehouse_store::governed_tag::delete_key(pool(&state)?, &key).await {
        Ok(()) => {}
        Err(lakehouse_store::StoreError::NotFound) => {
            return Err(tag_key_not_found(&key).into());
        }
        // QA `LH-DATA-12-15` (`DATA-54`): only tables the caller may read are
        // named, the rest are counted. A catalog that cannot be read keeps the
        // store's sentence, which still counts them, and the refusal stands.
        Err(lakehouse_store::StoreError::Validation(counted)) => {
            let ids = lakehouse_store::governed_tag::assets_with_key(pool(&state)?, &key).await;
            let sentence = match ids {
                Ok(ids) => {
                    match crate::routes::catalog::readable_asset_names(&state, &principal, &ids)
                        .await
                    {
                        Some(names) => tag_key_in_use_sentence(&key, ids.len(), &names),
                        None => counted,
                    }
                }
                Err(err) => {
                    tracing::warn!(
                        %err,
                        "the tables holding a tag key could not be read"
                    );
                    counted
                }
            };
            return Err(ApiError::BadRequest(sentence).into());
        }
        Err(other) => return Err(ApiError::from(other).into()),
    }
    audit_tag_key(
        &state,
        &principal,
        &key,
        "governance.tag_key_delete",
        json!({}),
    )
    .await;
    Ok(ApiJson(json!({ "ok": true, "key": key })))
}

/// The refusal to delete a tag key in use: the count of every table that
/// holds it, then up to three of them by name and "and N more" for the rest
/// (QA `LH-DATA-12-15`, `DATA-54`). `names` are the ones the caller may read;
/// `total` is the count of all of them, so a caller never learns of a table
/// it may not read.
fn tag_key_in_use_sentence(key: &str, total: usize, names: &[String]) -> String {
    let (noun, them) = if total == 1 {
        ("table", "that table")
    } else {
        ("tables", "those tables")
    };
    let head = format!("\"{key}\" is still given to {total} {noun}");
    if names.is_empty() {
        return format!("{head}. Remove it from {them} first.");
    }
    let shown: Vec<&str> = names.iter().take(3).map(String::as_str).collect();
    let rest = total.saturating_sub(shown.len());
    let list = if rest > 0 {
        format!("{} and {rest} more", shown.join(", "))
    } else {
        shown.join(", ")
    };
    format!("{head}: {list}. Remove it from {them} first.")
}

fn tag_key_not_found(key: &str) -> ApiError {
    ApiError::NotFound(format!("the tag key \"{key}\" is not defined"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    // QA `LH-DATA-12-15` (`DATA-54`): the refusal names up to three tables
    // and counts the rest; with no table to name it says which forms to
    // remove the key from.
    #[test]
    fn the_refusal_names_up_to_three_tables_and_counts_the_rest() {
        let names = ["A", "B", "C", "D"].map(str::to_owned);
        assert_eq!(
            tag_key_in_use_sentence("data_domain", 5, &names),
            "\"data_domain\" is still given to 5 tables: A, B, C and 2 more. \
             Remove it from those tables first."
        );
        assert_eq!(
            tag_key_in_use_sentence("data_domain", 1, &[]),
            "\"data_domain\" is still given to 1 table. Remove it from that table first."
        );
    }

    /// `WEB-8` S2 T8 (D8): the grammar route's answer is what the validator
    /// accepts. Every function and operator the route names is accepted in a
    /// filter, and a function and an operator it does not name are refused —
    /// which is why the route reads the validator's own constants instead of
    /// carrying a list of its own.
    #[tokio::test]
    async fn row_filter_grammar_names_only_what_the_validator_accepts() {
        use crate::sql_rewrite::{Engine, validate_row_filter_expr};

        let columns = vec!["a".to_owned(), "b".to_owned()];
        let grammar = row_filter_grammar()
            .await
            .expect("the route has no failure path")
            .0;

        for function in grammar.functions {
            let filter = format!("{function}(a) = 1");
            assert!(
                validate_row_filter_expr(&filter, &columns, Engine::ClickHouse).is_ok(),
                "the route names function '{function}' but the validator refuses '{filter}'"
            );
        }
        for operator in grammar.operators {
            let filter = match *operator {
                "=" => "a = 1".to_owned(),
                "<>" => "a <> 1".to_owned(),
                "<" => "a < 1".to_owned(),
                "<=" => "a <= 1".to_owned(),
                ">" => "a > 1".to_owned(),
                ">=" => "a >= 1".to_owned(),
                "AND" => "a = 1 AND b = 2".to_owned(),
                "OR" => "a = 1 OR b = 2".to_owned(),
                "+" => "a + 1 = 2".to_owned(),
                "-" => "a - 1 = 0".to_owned(),
                "*" => "a * 2 = 2".to_owned(),
                "/" => "a / 2 = 1".to_owned(),
                "%" => "a % 2 = 1".to_owned(),
                "NOT" => "NOT a = 1".to_owned(),
                other => panic!("the route names operator '{other}' with no filter in this test"),
            };
            assert!(
                validate_row_filter_expr(&filter, &columns, Engine::ClickHouse).is_ok(),
                "the route names operator '{operator}' but the validator refuses '{filter}'"
            );
        }

        assert!(
            validate_row_filter_expr("nope(a) = 1", &columns, Engine::ClickHouse).is_err(),
            "the validator accepts a function the route does not name"
        );
        assert!(
            validate_row_filter_expr("a || b", &columns, Engine::ClickHouse).is_err(),
            "the validator accepts an operator the route does not name"
        );

        // `WEB-8` S2 review round 1, S2: one filter per named form, checked
        // on both engines the validator knows — the forms are match arms, not
        // a membership list, so this loop is what ties the constant to the
        // arms. `a IS TRUE` is a shape the validator refuses and the route
        // does not name.
        for form in grammar.forms {
            let filter = match *form {
                "LIKE" => "a LIKE 'x%'",
                "ILIKE" => "a ILIKE 'x%'",
                "BETWEEN" => "a BETWEEN 1 AND 2",
                "IN" => "a IN (1, 2)",
                "IS NULL" => "a IS NULL",
                "IS NOT NULL" => "a IS NOT NULL",
                other => panic!("the route names form '{other}' with no filter in this test"),
            };
            for engine in [Engine::ClickHouse, Engine::Trino] {
                assert!(
                    validate_row_filter_expr(filter, &columns, engine).is_ok(),
                    "the route names form '{form}' but the validator refuses '{filter}' on {engine:?}"
                );
            }
        }
        assert!(
            validate_row_filter_expr("a IS TRUE", &columns, Engine::ClickHouse).is_err(),
            "the validator accepts a form the route does not name"
        );
    }

    #[test]
    fn create_policy_rejects_a_conditions_blob_with_no_real_obligation() {
        let body = json!({
            "name": "p1", "kind": "Row filter", "subjects": "s", "resources": "r",
            "effect": "Permit with obligation",
            "conditions": r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":[],"rowFilter":""}"#,
        });
        let err = create_policy_body(body).expect_err("empty obligation must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[test]
    fn create_policy_accepts_a_well_formed_conditions_blob() {
        let body = json!({
            "name": "p2", "kind": "Row filter", "subjects": "s", "resources": "r",
            "effect": "Permit with obligation",
            "conditions": r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#,
        });
        assert!(create_policy_body(body).is_ok());
    }

    #[test]
    fn create_policy_accepts_legacy_prose_conditions_unchanged() {
        let body = json!({
            "name": "p3", "kind": "Row filter", "subjects": "All analysts",
            "resources": "tenant-scoped tables", "effect": "Permit with obligation",
            "conditions": "Applies broadly, reviewed quarterly",
        });
        assert!(create_policy_body(body).is_ok());
    }

    #[test]
    fn create_policy_accepts_absent_conditions() {
        let body = json!({
            "name": "p4", "kind": "Row filter", "subjects": "s", "resources": "r",
            "effect": "Permit with obligation",
        });
        assert!(create_policy_body(body).is_ok());
    }

    // ── SEC-61: the policy list says when a policy enforces nothing ─────

    /// A stored policy row, as `lakehouse-store` returns it; the kind and
    /// conditions are what each test varies.
    fn stored_policy(kind: &str, conditions: Option<&str>) -> Policy {
        Policy {
            id: "11111111-1111-1111-1111-111111111111".to_owned(),
            name: "mask_customer_email".to_owned(),
            status: "ready".to_owned(),
            kind: kind.to_owned(),
            subjects: "Analyst".to_owned(),
            resources: "serving.mart_customer".to_owned(),
            effect: "Permit with obligation".to_owned(),
            version: 1,
            owner: "Rina".to_owned(),
            updated_at: "2026-11-04T00:00:00.000Z".to_owned(),
            conditions: conditions.map(str::to_owned),
        }
    }

    #[test]
    fn a_policy_with_structured_conditions_is_enforceable() {
        let view = PolicyView::new(
            stored_policy(
                "Row filter",
                Some(r#"{"roles":["Analyst"],"table":"serving.mart_customer","mask":["email"]}"#),
            ),
            None,
        );
        assert!(view.enforceable);
    }

    #[test]
    fn free_text_conditions_are_not_enforceable() {
        let view = PolicyView::new(
            stored_policy("Row filter", Some("Applies broadly, reviewed quarterly")),
            None,
        );
        assert!(
            !view.enforceable,
            "the engine skips prose, so the list must not call it enforced"
        );
    }

    #[test]
    fn a_policy_with_no_conditions_is_not_enforceable() {
        let view = PolicyView::new(stored_policy("Retention", None), None);
        assert!(!view.enforceable);
    }

    #[test]
    fn an_agent_autonomy_policy_is_not_enforceable() {
        // Agent autonomy is enforced by nothing in the workspace; free-text
        // prose is all such a policy can carry today.
        let view = PolicyView::new(
            stored_policy(
                "Agent autonomy",
                Some("Agents may act without approval on serving tables"),
            ),
            None,
        );
        assert!(!view.enforceable);
    }

    #[test]
    fn the_policy_wire_shape_keeps_the_row_and_adds_enforceable() {
        let view = PolicyView::new(
            stored_policy(
                "Row filter",
                Some(r#"{"roles":["Analyst"],"table":"serving.mart_customer","mask":["email"]}"#),
            ),
            None,
        );
        let wire = serde_json::to_value(&view).expect("a PolicyView serializes");
        assert_eq!(wire["enforceable"], json!(true));
        assert_eq!(wire["name"], json!("mask_customer_email"));
        assert_eq!(wire["updatedAt"], json!("2026-11-04T00:00:00.000Z"));
        assert_eq!(wire["status"], json!("ready"));
    }

    // ── WEB-3 S3a-D4: no invented classification ────────────────────────

    #[test]
    fn a_registry_tier_that_is_a_level_is_reported_as_that_level() {
        let row = registry_classification("orders", "commerce_orders", "confidential")
            .expect("a level is reported");
        assert_eq!(
            row["classification"],
            json!("confidential"),
            "the dataset's own tier, never a hardcoded \"internal\": {row}"
        );
        assert_eq!(row["id"], json!("c-orders"));
        assert_eq!(row["confidence"], json!(1));
        assert_eq!(row["reviewStatus"], json!("auto"));
    }

    /// The key the stored classification rules use for a Bronze table
    /// (`lineageKey`), so the console can match the two.
    #[test]
    fn a_registry_row_is_keyed_the_way_stored_rules_key_a_table() {
        assert_eq!(
            registry_classification("orders", "commerce_orders", "internal")
                .expect("a level is reported")["asset"],
            json!("bronze.commerce_orders")
        );
        // No table name in the registry: the slug is what the catalog shows,
        // and what a rule written on that page uses.
        assert_eq!(
            registry_classification("orders", "", "internal").expect("a level is reported")["asset"],
            json!("orders")
        );
    }

    /// Every seeded dataset carries `primer` (and the secondary registry
    /// `sekunder`), which is where data comes from, not how sensitive it is:
    /// reporting a level for it would be an invented value.
    #[test]
    fn a_source_tier_is_not_a_classification_and_reports_nothing() {
        for tier in ["primer", "sekunder", "", "PRIMER", "internal-ish"] {
            assert_eq!(
                registry_classification("orders", "commerce_orders", tier),
                None,
                "{tier} names no classification level"
            );
        }
    }

    #[test]
    fn every_product_level_is_accepted() {
        for level in CLASSIFICATION_LEVELS {
            assert!(
                registry_classification("slug", "table", level).is_some(),
                "{level} is one of the console's levels"
            );
        }
    }

    // ── WEB-3 S3a-D1: a policy says which table page it belongs to ──────

    #[test]
    fn a_silver_or_serving_table_is_its_own_asset_id() {
        for table in ["serving.mart_customer", "silver.orders_enriched"] {
            let conditions =
                format!(r#"{{"roles":["Analyst"],"table":"{table}","mask":["email"]}}"#);
            assert_eq!(
                PolicyAsset::of(Some(&conditions)),
                Some(PolicyAsset::Table(table.to_owned())),
                "{table}"
            );
        }
    }

    #[test]
    fn a_bronze_table_asks_the_registry_for_its_slug() {
        assert_eq!(
            PolicyAsset::of(Some(
                r#"{"roles":["Analyst"],"table":"bronze.commerce_orders","mask":["card_last4"]}"#
            )),
            Some(PolicyAsset::Bronze("commerce_orders".to_owned()))
        );
    }

    #[test]
    fn absent_conditions_name_no_asset() {
        assert_eq!(PolicyAsset::of(None), None);
    }

    #[test]
    fn free_text_conditions_name_no_asset() {
        assert_eq!(
            PolicyAsset::of(Some("Applies broadly, reviewed quarterly")),
            None
        );
    }

    /// The third unparseable shape: JSON-looking but truncated. The engine's
    /// parser gives up rather than guessing, so no table is named and no
    /// asset id is invented from a substring.
    #[test]
    fn malformed_conditions_name_no_asset() {
        for conditions in [
            r#"{"roles":["Analyst"],"table":"serving.mart_customer","#,
            r#"{"roles":["Analyst"],"table":}"#,
            "{",
        ] {
            assert_eq!(PolicyAsset::of(Some(conditions)), None, "{conditions}");
        }
    }

    #[test]
    fn a_condition_with_no_obligation_names_no_asset() {
        // JSON-shaped, but the engine would enforce nothing from it, so the
        // table it mentions is not a fact about a policy in force.
        assert_eq!(
            PolicyAsset::of(Some(
                r#"{"roles":["Analyst"],"table":"serving.mart_customer","mask":[]}"#
            )),
            None
        );
    }

    #[test]
    fn a_table_no_zone_addresses_name_no_asset() {
        for conditions in [
            r#"{"roles":["Analyst"],"table":"core.customer_360","mask":["email"]}"#,
            r#"{"roles":["Analyst"],"table":"orders","mask":["email"]}"#,
            r#"{"roles":["Analyst"],"table":"bronze.","mask":["email"]}"#,
        ] {
            assert_eq!(PolicyAsset::of(Some(conditions)), None, "{conditions}");
        }
    }

    #[test]
    fn the_wire_shape_omits_an_unknown_asset_id_and_keeps_a_known_one() {
        let conditions =
            r#"{"roles":["Analyst"],"table":"serving.mart_customer","mask":["email"]}"#;
        let known = PolicyView::new(
            stored_policy("Row filter", Some(conditions)),
            Some("serving.mart_customer".to_owned()),
        );
        let wire = serde_json::to_value(&known).expect("a PolicyView serializes");
        assert_eq!(wire["assetId"], json!("serving.mart_customer"));

        let unknown = PolicyView::new(stored_policy("Row filter", Some(conditions)), None);
        let wire = serde_json::to_value(&unknown).expect("a PolicyView serializes");
        assert!(
            wire.get("assetId").is_none(),
            "an unknown asset id is absent, never a fabricated value"
        );
    }

    // ── WS7 item A5: real policy impact preview ─────────────────────────

    #[test]
    fn preview_body_reports_unmeasured_when_conditions_do_not_parse() {
        let v = preview_result_json(None, None);
        assert!(v["affectedRows"].is_null());
        assert_eq!(
            v["maskedColumns"].as_array().unwrap().as_slice(),
            &[] as &[serde_json::Value]
        );
        assert_eq!(v["supported"], json!(false));
    }

    #[test]
    fn preview_body_reports_real_values_when_measured() {
        let v = preview_result_json(Some(42), Some(vec!["email".to_owned()]));
        assert_eq!(v["affectedRows"], json!(42));
        assert_eq!(v["maskedColumns"], json!(["email"]));
        assert_eq!(v["supported"], json!(true));
    }

    #[test]
    fn preview_body_is_unsupported_when_only_the_row_count_is_missing() {
        // The masked-column cross-check succeeded but the row-count
        // measurement did not (a bad row filter, or ClickHouse timed
        // out) — `supported` must still be false: a HALF-real preview is
        // not a real one.
        let v = preview_result_json(None, Some(vec!["email".to_owned()]));
        assert_eq!(v["maskedColumns"], json!(["email"]));
        assert_eq!(v["supported"], json!(false));
    }

    fn maintenance_fixture_row() -> Map<String, Value> {
        let mut row = Map::new();
        row.insert("table_name".to_owned(), json!("orders"));
        row.insert("run_at".to_owned(), json!("2026-01-01T00:00:00Z"));
        row.insert("dry_data".to_owned(), json!("3"));
        row.insert("dry_manifests".to_owned(), json!("1"));
        row.insert("applied_data".to_owned(), json!("2"));
        row.insert("applied_manifests".to_owned(), json!("0"));
        row.insert("skipped_verbs".to_owned(), json!(""));
        row
    }

    #[test]
    fn maintenance_run_row_json_matches_the_existing_per_row_shape() {
        let row = maintenance_fixture_row();

        let value = maintenance_run_row_json(&row);

        assert_eq!(
            value,
            json!({
                "tableName": "orders",
                "runAt": "2026-01-01T00:00:00Z",
                "dryRun": {
                    "deletedDataFiles": "3",
                    "deletedManifestFiles": "1",
                },
                "applied": {
                    "deletedDataFiles": "2",
                    "deletedManifestFiles": "0",
                },
                "skippedVerbs": "",
            })
        );
    }

    #[test]
    fn maintenance_run_query_for_all_tables_has_no_where_and_keeps_limit_500() {
        let sql = maintenance_run_query(None);

        assert!(!sql.contains("WHERE"));
        assert!(sql.contains("ORDER BY run_at DESC LIMIT 500"));
    }

    #[test]
    fn maintenance_run_query_for_one_table_filters_and_limits_to_one() {
        let sql = maintenance_run_query(Some("orders"));

        assert!(sql.contains("WHERE table_name = 'orders'"));
        assert!(sql.contains("ORDER BY run_at DESC LIMIT 1"));
        assert!(!sql.contains("LIMIT 500"));
    }

    #[test]
    fn maintenance_run_query_escapes_a_quote_in_the_table_name() {
        let sql = maintenance_run_query(Some("o'rders"));

        assert!(sql.contains("WHERE table_name = 'o''rders'"));
    }

    /// `OPS-9`: the per-table read narrows on the qualified name the
    /// maintenance job records ([`maintenance_history_table_name`]), which
    /// is what makes a Bronze table's last run visible at all.
    #[test]
    fn maintenance_run_query_for_one_bronze_table_filters_the_qualified_name() {
        let sql = maintenance_run_query(Some(&maintenance_history_table_name("bronze", "orders")));

        assert!(sql.contains("WHERE table_name = 'bronze.orders'"));
        assert!(sql.contains("ORDER BY run_at DESC LIMIT 1"));
    }

    /// `OPS-9`, on the wire: the narrowed query `ClickHouse` receives is
    /// the qualified name, proven by a mock that only answers that SQL.
    #[tokio::test]
    async fn latest_maintenance_run_for_one_table_sends_the_qualified_recorded_name() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::body_string_contains(
                "table_name = 'bronze.orders'",
            ))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": [{
                    "table_name": "bronze.orders",
                    "run_at": "2026-01-01T00:00:00Z",
                    "dry_data": "0", "dry_manifests": "0",
                    "applied_data": "1", "applied_manifests": "0",
                    "skipped_verbs": "",
                }],
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let rows = latest_maintenance_run(
            &ch,
            Some(&maintenance_history_table_name("bronze", "orders")),
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["tableName"], json!("bronze.orders"));
    }

    /// Real precedent, cited in this task's plan: `lakehouse-clickhouse/
    /// src/lib.rs:401+`'s `wiremock`-backed `ChClient` tests — this
    /// module has no live-`ClickHouse` integration test of its own, so
    /// `ingest_runs_for_connector` is proven against a mocked HTTP
    /// response, not a real cluster.
    #[tokio::test]
    async fn ingest_runs_returns_only_the_requested_connectors_rows() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": [{
                    "connector_id": "conn-x", "job": "ingest_job", "object": "orders",
                    "rows": 42, "started_at": "2026-09-11T00:00:00Z",
                    "ended_at": "2026-09-11T00:00:05Z", "status": "succeeded", "error": "",
                }],
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let rows = ingest_runs_for_connector(&ch, "conn-x").await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].connector_id, "conn-x");
        assert_eq!(rows[0].job, "ingest_job");
        assert_eq!(rows[0].object, "orders");
        assert_eq!(rows[0].rows, Some(42));
        assert_eq!(rows[0].status, "succeeded");
    }

    /// WS3 plan review Z9: a `NULL` `rows` column (dlt's row count was
    /// genuinely unmeasured) must deserialize to `None`, never a
    /// fabricated `0` — [`nullable_u64_col`] is the guard, this is its
    /// integration-shaped proof against the same mocked envelope above.
    #[tokio::test]
    async fn ingest_runs_reports_a_null_row_count_as_none_not_zero() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "data": [{
                    "connector_id": "conn-x", "job": "ingest_job", "object": "orders",
                    "rows": null, "started_at": "2026-09-11T00:00:00Z",
                    "ended_at": "2026-09-11T00:00:05Z", "status": "rejected", "error": "ssrf blocked",
                }],
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let rows = ingest_runs_for_connector(&ch, "conn-x").await.unwrap();

        assert_eq!(
            rows[0].rows, None,
            "a NULL rows column must never become a fabricated 0"
        );
    }

    /// Before any run has recorded an outcome, `bronze_meta.ingest_run`
    /// does not exist yet: that is "no runs", not an outage.
    #[tokio::test]
    async fn ingest_runs_or_empty_reports_no_runs_before_the_table_exists() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(404).set_body_string(
                "Code: 60. DB::Exception: Unknown table expression identifier \
                 'lake.bronze_meta.ingest_run' in scope SELECT connector_id FROM \
                 lake.`bronze_meta.ingest_run`. (UNKNOWN_TABLE)",
            ))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        let rows = ingest_runs_or_empty(&ch, "conn-x").await.unwrap();

        assert_eq!(rows, [] as [crate::routes::governance::IngestRunRow; 0]);
    }

    #[tokio::test]
    async fn ingest_runs_or_empty_still_surfaces_any_other_failure() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(500)
                    .set_body_string("Code: 210. DB::NetException: Connection refused"),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());

        assert!(ingest_runs_or_empty(&ch, "conn-x").await.is_err());
    }

    #[test]
    fn nullable_u64_col_accepts_a_quoted_string_a_bare_number_and_null() {
        let mut row = Map::new();
        row.insert("as_string".to_owned(), json!("42"));
        row.insert("as_number".to_owned(), json!(42));
        row.insert("as_null".to_owned(), Value::Null);

        assert_eq!(nullable_u64_col(&row, "as_string"), Some(42));
        assert_eq!(nullable_u64_col(&row, "as_number"), Some(42));
        assert_eq!(nullable_u64_col(&row, "as_null"), None);
        assert_eq!(nullable_u64_col(&row, "missing"), None);
    }

    /// `Query<IngestRunsQuery>` extraction fails (axum's own 400) when
    /// `connectorId` is absent — asserted the same way this module's
    /// other dedicated route (`lineage`) is unit-tested: against the
    /// route's own logic with a minimal router, not the full
    /// Postgres-backed `TestApp` `tests/route_auth.rs` uses (no
    /// `ClickHouse`/Postgres call happens before extraction fails).
    #[tokio::test]
    async fn ingest_runs_route_requires_connector_id() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let cfg = crate::config::Config::from_map(&std::collections::HashMap::new()).unwrap();
        let router = axum::Router::new()
            .route(
                "/api/governance/ingest-runs",
                axum::routing::get(ingest_runs),
            )
            .with_state(AppState::new(cfg));

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/governance/ingest-runs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn kind_parses_known_values() {
        assert_eq!(Kind::parse("quality"), Kind::Quality);
        assert_eq!(Kind::parse("audit"), Kind::Audit);
        assert_eq!(Kind::parse("classification"), Kind::Classification);
        assert_eq!(Kind::parse("residency"), Kind::Residency);
        assert_eq!(Kind::parse("maintenance"), Kind::Maintenance);
        assert_eq!(Kind::parse("replication"), Kind::Replication);
    }

    #[test]
    fn kind_parse_unknown_falls_back() {
        assert_eq!(Kind::parse("bogus-kind"), Kind::Unknown);
        // `lineage` is routed to a dedicated handler and must never reach
        // this dispatch; if it somehow did, it should NOT be treated as a
        // recognized governance kind.
        assert_eq!(Kind::parse("lineage"), Kind::Unknown);
    }

    /// No quality job has run on this stack yet: the whole `_silver_meta`
    /// database is missing, which is "no verdicts", not an outage.
    #[test]
    fn a_missing_quality_database_or_table_is_no_verdicts_yet() {
        assert!(is_missing_quality_source(
            "Code: 81. DB::Exception: Database _silver_meta does not exist. \
             (UNKNOWN_DATABASE) (version 26.8.9.10 (official build))"
        ));
        assert!(is_missing_quality_source(
            "Code: 60. DB::Exception: Unknown table expression identifier \
             '_silver_meta.quality' in scope SELECT 1. (UNKNOWN_TABLE)"
        ));
        assert!(!is_missing_quality_source(
            "Code: 241. DB::Exception: Memory limit exceeded. (MEMORY_LIMIT_EXCEEDED)"
        ));
    }

    #[test]
    fn status_of_maps_verdicts() {
        assert_eq!(status_of("fail"), "failed");
        assert_eq!(status_of("warn"), "warning");
        assert_eq!(status_of("pass"), "passed");
        assert_eq!(status_of("anything-else"), "passed");
    }

    #[test]
    fn severity_of_maps_verdicts() {
        assert_eq!(severity_of("fail"), "high");
        assert_eq!(severity_of("warn"), "medium");
        assert_eq!(severity_of("pass"), "info");
    }

    /// The principal the auth gate always inserts before [`get`] runs. An
    /// administrator is the right caller here: `replication` is not narrowed
    /// by role (`GOV-15` covers the quality rules only). `tag_key_handlers`'s
    /// own `admin()` lives in a sibling module and is not reachable from here,
    /// so this is the same principal inline.
    fn replication_admin() -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "Rina Admin".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("governance:write,*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// `GET /api/governance/replication` against a `ClickHouse` stub that
    /// answers every query with `status` and `body`; returns the route's
    /// status and JSON body.
    async fn replication_route_answer(ch: Option<(u16, &str)>) -> (StatusCode, Value) {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let server = wiremock::MockServer::start().await;
        let mut env = std::collections::HashMap::new();
        match ch {
            Some((status, body)) => {
                wiremock::Mock::given(wiremock::matchers::method("POST"))
                    .respond_with(
                        wiremock::ResponseTemplate::new(status).set_body_string(body.to_owned()),
                    )
                    .mount(&server)
                    .await;
                env.insert("CH_URL".to_owned(), server.uri());
            }
            // Nothing listens on port 1: `ClickHouse` is down.
            None => {
                env.insert("CH_URL".to_owned(), "http://127.0.0.1:1".to_owned());
            }
        }
        let cfg = crate::config::Config::from_map(&env).unwrap();
        let router = axum::Router::new()
            .route("/api/governance/{kind}", axum::routing::get(get))
            .layer(Extension(replication_admin()))
            .with_state(AppState::new(cfg));

        let resp = router
            .oneshot(
                Request::builder()
                    .uri("/api/governance/replication")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// WEB-3 review R1: before any CDC connector has run,
    /// `bronze_meta.replication_slot` does not exist. That is "no slots", a
    /// 200 with an empty list, not an outage on a fresh install.
    #[tokio::test]
    async fn replication_route_answers_an_empty_list_when_the_slot_table_does_not_exist() {
        let (status, body) = replication_route_answer(Some((
            404,
            "Code: 60. DB::Exception: Unknown table expression identifier \
             'lake.bronze_meta.replication_slot' in scope SELECT connector_id FROM \
             lake.`bronze_meta.replication_slot`. (UNKNOWN_TABLE)",
        )))
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "replicationSlots": [] }));
    }

    /// WEB-3 review R1: the empty list is for the missing table only.
    /// `ClickHouse` being unreachable is still a 503.
    #[tokio::test]
    async fn replication_route_is_503_when_clickhouse_is_down() {
        let (status, body) = replication_route_answer(None).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(body.get("replicationSlots").is_none());
    }

    /// WEB-3 review R1: another engine error (here a permission error,
    /// `ACCESS_DENIED`, code 497) is still a 503, and a body that merely
    /// mentions `UNKNOWN_TABLE` without the code does not count: the
    /// decision is the code, never the text.
    #[tokio::test]
    async fn replication_route_is_503_for_any_other_clickhouse_error() {
        for body in [
            "Code: 497. DB::Exception: default: Not enough privileges. (ACCESS_DENIED)",
            "UNKNOWN_TABLE",
        ] {
            let (status, _) = replication_route_answer(Some((500, body))).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
        }
    }

    /// The bug this guards: `active` is `UInt8`, so `ClickHouse` renders it as
    /// a JSON NUMBER. `str_col` is `Value::as_str` -> `None` for a number ->
    /// `""` -> false, and every replication slot showed as disconnected on
    /// the Ingestion page, healthy ones included. Only `toString(active)` in
    /// the query makes the value a string this can read at all.
    #[test]
    fn replication_query_stringifies_every_numeric_column() {
        for col in ["active", "wal_retained_bytes", "confirmed_flush_lag_bytes"] {
            assert!(
                REPLICATION_SLOTS_SQL.contains(&format!("toString({col})")),
                "{col} is numeric in ClickHouse and must be wrapped in toString(), \
                 or str_col reads it as an empty string"
            );
        }
    }

    #[test]
    fn ch_bool_reads_a_stringified_uint8() {
        let mut row = serde_json::Map::new();
        row.insert("active".into(), Value::from("1"));
        assert!(ch_bool(&row, "active"));
        row.insert("active".into(), Value::from("0"));
        assert!(!ch_bool(&row, "active"));
        // A future Bool column stringifies as "true"/"false".
        row.insert("active".into(), Value::from("true"));
        assert!(ch_bool(&row, "active"));
        row.insert("active".into(), Value::from("false"));
        assert!(!ch_bool(&row, "active"));
    }

    /// The failure mode itself: an UNWRAPPED numeric column arrives as a JSON
    /// number and reads false, whatever its real value. This asserts the
    /// broken behaviour deliberately, so the reason the query must wrap the
    /// column is documented in an executable form rather than only in a
    /// comment.
    #[test]
    fn ch_bool_cannot_read_a_bare_numeric_column() {
        let mut row = serde_json::Map::new();
        row.insert("active".into(), Value::from(1));
        assert!(
            !ch_bool(&row, "active"),
            "a bare UInt8 arrives as a JSON number and is unreadable as a string — \
             this is why the query wraps it"
        );
    }

    #[test]
    fn ch_bool_is_false_for_a_missing_column() {
        assert!(!ch_bool(&serde_json::Map::new(), "active"));
    }

    #[test]
    fn actor_kind_of_narrows_copilot_principal_kinds_to_agent() {
        assert_eq!(actor_kind_of(Some("user")), "user");
        assert_eq!(actor_kind_of(Some("service")), "service");
        // Neither "copilot" nor "schedule" is a value the frontend's
        // `ActorKind` union knows; both must collapse to "agent" rather
        // than leak an unrecognized string into a typed field.
        assert_eq!(actor_kind_of(Some("copilot")), "agent");
        assert_eq!(actor_kind_of(Some("schedule")), "agent");
        // Changed by SEC-70 D4: this line asserted "agent" until the QA case
        // showed the list handing a pre-sign-in lockout to the assistant.
        // With no recorded kind nobody signed in, so the platform recorded
        // the event: a service.
        assert_eq!(actor_kind_of(None), "service");
    }

    #[test]
    fn source_of_follows_the_principal_kind() {
        assert_eq!(source_of(Some("user")), "console");
        assert_eq!(source_of(Some("copilot")), "copilot");
        assert_eq!(source_of(Some("schedule")), "schedule");
        assert_eq!(source_of(Some("service")), "service");
        // Changed by SEC-70 D4: this line asserted "copilot" until the QA
        // case showed an event nobody signed in caused listed as the
        // assistant's. No recorded kind is the system.
        assert_eq!(source_of(None), "system");
    }

    #[test]
    fn actor_of_names_the_label_then_the_id_then_the_assistant_only_for_a_copilot_event() {
        // A label is the name, whatever the kind: that is how an event
        // nobody signed in caused names itself (SEC-70 D3).
        assert_eq!(
            actor_of(Some("Sign-in form"), None, None),
            "Sign-in form".to_owned()
        );
        assert_eq!(
            actor_of(Some("Sign-in form"), Some("u_1"), Some("user")),
            "Sign-in form".to_owned()
        );
        // No label: the principal id is what the row knows.
        assert_eq!(actor_of(None, Some("u_1"), Some("user")), "u_1".to_owned());
        // The one name for the assistant is only for a copilot event (AI-23),
        // and a copilot event with a label keeps the label.
        assert_eq!(
            actor_of(None, None, Some("copilot")),
            lakehouse_store::agents::COPILOT_EMPLOYEE_NAME.to_owned()
        );
        // Nothing at all: a row stored before SEC-70, or before the label.
        assert_eq!(actor_of(None, None, None), "System".to_owned());
        assert_eq!(
            actor_of(None, None, Some("service")),
            "System".to_owned(),
            "a service with no name is not the assistant"
        );
    }

    #[test]
    fn detail_of_a_grant_names_the_permission_the_grantee_and_the_day_it_ends() {
        let args = json!({
            "permission": "query:read",
            "grantee": "QA Approver <qa-approver@example.test>",
            "expiresAt": "2026-12-06T08:00:00Z",
            "unrelated": "never echoed",
        });
        let detail = detail_of("catalog.grant", &args).expect("a grant has a detail");
        assert_eq!(
            detail,
            "Granted query:read to QA Approver <qa-approver@example.test> until 2026-12-06"
        );
        assert!(!detail.contains("never echoed"));
    }

    #[test]
    fn detail_of_a_grant_without_an_expiry_does_not_invent_a_date() {
        let args = json!({ "permission": "query:read", "grantee": "A <a@example.test>" });
        let detail = detail_of("catalog.grant", &args).expect("a grant has a detail");
        assert_eq!(
            detail,
            "Granted query:read to A <a@example.test> until its expiry"
        );
    }

    #[test]
    fn detail_of_a_revoke_names_the_permission_and_the_grantee() {
        let args = json!({ "permission": "query:read", "grantee": "A <a@example.test>" });
        assert_eq!(
            detail_of("catalog.revoke", &args).as_deref(),
            Some("Revoked query:read from A <a@example.test>")
        );
    }

    #[test]
    fn detail_of_any_other_action_is_none() {
        assert_eq!(
            detail_of("catalog.annotate", &json!({ "fields": ["x"] })),
            None
        );
    }

    #[test]
    fn outcome_of_reports_a_gate_still_waiting_as_pending() {
        assert_eq!(outcome_of("allowed"), "success");
        assert_eq!(outcome_of("executed"), "success");
        assert_eq!(outcome_of("approved"), "success");
        assert_eq!(outcome_of("failed"), "error");
        assert_eq!(outcome_of("refused"), "denied");
        assert_eq!(outcome_of("rejected"), "denied");
        // The regression this test guards: a gate decision that is still
        // waiting reads "pending" — never "denied", because the policy
        // refused nothing, and never "success", because nobody has
        // decided yet.
        assert_eq!(outcome_of("needs_confirmation"), "pending");
        assert_eq!(outcome_of("needs_approval"), "pending");
    }

    /// `GOV-5`: a row whose decision is still open reports `"pending"` as
    /// its policy decision as well — it allowed nothing and denied
    /// nothing.
    #[test]
    fn policy_decision_of_reports_pending_for_a_gate_still_waiting() {
        assert_eq!(policy_decision_of("allowed"), "allow");
        assert_eq!(policy_decision_of("executed"), "allow");
        assert_eq!(policy_decision_of("approved"), "allow");
        assert_eq!(policy_decision_of("needs_confirmation"), "pending");
        assert_eq!(policy_decision_of("needs_approval"), "pending");
        assert_eq!(policy_decision_of("refused"), "deny");
        assert_eq!(policy_decision_of("rejected"), "deny");
        assert_eq!(policy_decision_of("failed"), "deny");
    }

    /// The merge sort in [`audit`] compares `at` strings lexicographically;
    /// this is the regression test for that assumption holding across rows
    /// from both sources (`Dagster`-derived and `audit_event`-derived use
    /// the same `YYYY-MM-DDTHH:MM:SS.mmmZ` format).
    #[test]
    fn audit_rows_sort_newest_first_by_at_string() {
        let mut rows = [
            json!({ "id": "a", "at": "2026-09-08T10:00:00.000Z" }),
            json!({ "id": "b", "at": "2026-09-08T12:00:00.000Z" }),
            json!({ "id": "c", "at": "2026-09-08T11:00:00.000Z" }),
        ];
        rows.sort_by(|a, b| {
            let a_at = a.get("at").and_then(Value::as_str).unwrap_or_default();
            let b_at = b.get("at").and_then(Value::as_str).unwrap_or_default();
            b_at.cmp(a_at)
        });
        let ids: Vec<&str> = rows
            .iter()
            .map(|r| r.get("id").and_then(Value::as_str).unwrap())
            .collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    // ── DATA-12 part 2: governed tag keys ────────────────────────────────
    mod tag_key_handlers {
        #![allow(clippy::unwrap_used, clippy::expect_used)]

        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
        // testcontainer bootstrap runs for this test binary.
        use lakehouse_test_support as _;
        use uuid::Uuid;

        use super::super::*;
        use crate::config::Config;
        use crate::error::ApiRejection;

        fn state_for(pool: &lakehouse_store::PgPool) -> AppState {
            let options = pool.connect_options();
            let mut env = HashMap::new();
            env.insert(
                "DATABASE_URL".to_owned(),
                format!(
                    "postgres://{}:postgres@{}:{}/{}",
                    options.get_username(),
                    options.get_host(),
                    options.get_port(),
                    options.get_database().expect("a named database"),
                ),
            );
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        // PR #106 review R2 BLOCKER 8: happy-path tests use an unrestricted (*:*) caller
        // so the fail-closed catalog_tenant_refusal gate allows tag key writes.
        fn admin() -> Principal {
            Principal {
                id: PrincipalId::User(Uuid::new_v4()),
                tenant_ids: Vec::new(),
                display_name: "Rina Admin".to_owned(),
                permissions: PermissionSet::parse("governance:write,*:*"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            }
        }

        async fn answer<T: serde::Serialize>(
            result: Result<ApiJson<T>, ApiRejection>,
        ) -> (StatusCode, Value) {
            let response = match result {
                Ok(ok) => ok.into_response(),
                Err(rejection) => rejection.into_response(),
            };
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, serde_json::from_slice(&bytes).unwrap_or_default())
        }

        async fn create(state: &AppState, body: &str) -> (StatusCode, Value) {
            answer(
                create_tag_key(
                    State(state.clone()),
                    Extension(admin()),
                    HeaderMap::new(),
                    Bytes::copy_from_slice(body.as_bytes()),
                )
                .await,
            )
            .await
        }

        async fn update(state: &AppState, key: &str, body: &str) -> (StatusCode, Value) {
            answer(
                update_tag_key(
                    State(state.clone()),
                    Extension(admin()),
                    Path(key.to_owned()),
                    HeaderMap::new(),
                    Bytes::copy_from_slice(body.as_bytes()),
                )
                .await,
            )
            .await
        }

        async fn remove(state: &AppState, key: &str) -> (StatusCode, Value) {
            answer(
                delete_tag_key(
                    State(state.clone()),
                    Extension(admin()),
                    Path(key.to_owned()),
                    HeaderMap::new(),
                )
                .await,
            )
            .await
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_tag_key_is_defined_listed_widened_and_deleted_and_each_step_is_audited(
            pool: lakehouse_store::PgPool,
        ) {
            let state = state_for(&pool);
            let (status, made) = create(
                &state,
                r#"{"key":"data_domain","allowedValues":["finance","ops"]}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(made["key"], json!("data_domain"));
            assert_eq!(made["allowedValues"], json!(["finance", "ops"]));
            assert_eq!(made["createdBy"], json!("Rina Admin"));

            let (_, listed) = answer(
                list_tag_keys(State(state.clone()), Extension(admin()), HeaderMap::new()).await,
            )
            .await;
            assert_eq!(listed.as_array().unwrap().len(), 1);
            assert_eq!(listed[0]["allowedValues"], json!(["finance", "ops"]));

            let (status, widened) = update(
                &state,
                "data_domain",
                r#"{"allowedValues":["finance","ops","hr"]}"#,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(widened["allowedValues"], json!(["finance", "ops", "hr"]));

            let (status, gone) = remove(&state, "data_domain").await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(gone, json!({ "ok": true, "key": "data_domain" }));

            let actions: Vec<String> = sqlx::query_scalar(
                "SELECT action FROM audit_event WHERE resource_kind = 'tag_key' \
                 AND resource_id = 'data_domain' ORDER BY at",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(
                actions,
                vec![
                    "governance.tag_key_create",
                    "governance.tag_key_update",
                    "governance.tag_key_delete"
                ]
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn tag_key_refusals_name_the_field_and_store_nothing(pool: lakehouse_store::PgPool) {
            let state = state_for(&pool);
            let cases = [
                (r#"{"key":"Bad Key","allowedValues":["a"]}"#, "key"),
                (r#"{"key":"k","allowedValues":[]}"#, "allowed value"),
                (
                    r#"{"key":"k","allowedValues":["Finance"]}"#,
                    "allowed value",
                ),
                (r#"{"key":"k","allowedValues":["a","a"]}"#, "allowed value"),
                (r#"{"key":"k"}"#, "allowedValues"),
                ("not json", "invalid JSON"),
            ];
            for (body, names) in cases {
                let (status, answer) = create(&state, body).await;
                assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
                let message = answer["error"].as_str().unwrap();
                assert!(message.contains(names), "{body}: {message}");
            }
            let many: Vec<String> = (0..51).map(|i| format!("\"v{i}\"")).collect();
            let (status, answer) = create(
                &state,
                &format!(r#"{{"key":"k","allowedValues":[{}]}}"#, many.join(",")),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(answer["error"].as_str().unwrap().contains("at most 50"));

            let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM governed_tag_key")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(stored, 0);
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_duplicate_tag_key_is_409_a_missing_one_is_404_and_a_rename_is_refused(
            pool: lakehouse_store::PgPool,
        ) {
            let state = state_for(&pool);
            let body = r#"{"key":"tier","allowedValues":["gold"]}"#;
            assert_eq!(create(&state, body).await.0, StatusCode::OK);
            let (status, again) = create(&state, body).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert!(again["error"].as_str().unwrap().contains("\"tier\""));

            assert_eq!(
                update(&state, "nope", r#"{"allowedValues":["a"]}"#).await.0,
                StatusCode::NOT_FOUND
            );
            assert_eq!(remove(&state, "nope").await.0, StatusCode::NOT_FOUND);

            let (status, rename) = update(
                &state,
                "tier",
                r#"{"key":"level","allowedValues":["gold"]}"#,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(rename["error"].as_str().unwrap().contains("key"));
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_tag_key_in_use_cannot_lose_a_value_or_be_deleted_and_the_answer_counts_tables(
            pool: lakehouse_store::PgPool,
        ) {
            let state = state_for(&pool);
            create(
                &state,
                r#"{"key":"data_domain","allowedValues":["finance","ops"]}"#,
            )
            .await;
            for asset in ["silver.a", "silver.b"] {
                lakehouse_store::governed_tag::set_asset_tags(
                    &pool,
                    asset,
                    &[("data_domain".to_owned(), "ops".to_owned())],
                    "Rina",
                )
                .await
                .unwrap();
            }

            let (status, refused) =
                update(&state, "data_domain", r#"{"allowedValues":["finance"]}"#).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(refused["error"].as_str().unwrap().contains("\"ops\" (2)"));

            let (status, refused) = remove(&state, "data_domain").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(refused["error"].as_str().unwrap().contains("2 tables"));

            let kept: Vec<String> = sqlx::query_scalar(
                "SELECT unnest(allowed_values) FROM governed_tag_key ORDER BY 1",
            )
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(kept, vec!["finance", "ops"], "a refusal changes nothing");
        }

        // QA `LH-DATA-12-15` (`DATA-54`): the refusal names up to three of
        // the tables that hold the key, and only ones the caller may read.
        #[sqlx::test(migrations = "../../migrations")]
        async fn deleting_a_key_in_use_names_the_tables_that_hold_it(
            pool: lakehouse_store::PgPool,
        ) {
            let state = state_for(&pool);
            create(
                &state,
                r#"{"key":"data_domain","allowedValues":["finance","ops"]}"#,
            )
            .await;
            for asset in ["silver.a", "silver.b"] {
                lakehouse_store::governed_tag::set_asset_tags(
                    &pool,
                    asset,
                    &[("data_domain".to_owned(), "ops".to_owned())],
                    "Rina",
                )
                .await
                .unwrap();
            }
            crate::routes::catalog::prime_search_copy_for_test(
                &state,
                vec![
                    json!({ "id": "silver.a", "name": "Orders" }),
                    json!({ "id": "silver.b", "name": "Customers" }),
                ],
            )
            .await;

            let (status, refused) = remove(&state, "data_domain").await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(
                refused["error"]
                    .as_str()
                    .unwrap()
                    .contains("2 tables: Orders, Customers."),
                "{refused}"
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT count(*) FROM governed_tag_key")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1,
                "a refusal changes nothing"
            );
        }

        // Review round 1 R8 (`DATA-54`): a caller who may not open the catalog
        // is told how many tables hold the key, and named none of them.
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_caller_without_catalog_read_is_told_the_count_and_no_name(
            pool: lakehouse_store::PgPool,
        ) {
            let state = state_for(&pool);
            // A deployment with no tenant of the seeded ones: the catalog's
            // tenant rule then changes nothing, and the permissions are what
            // is under test.
            sqlx::query("DELETE FROM app_user_tenant")
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("DELETE FROM tenant")
                .execute(&pool)
                .await
                .unwrap();
            create(
                &state,
                r#"{"key":"data_domain","allowedValues":["finance","ops"]}"#,
            )
            .await;
            for asset in ["silver.a", "silver.b"] {
                lakehouse_store::governed_tag::set_asset_tags(
                    &pool,
                    asset,
                    &[("data_domain".to_owned(), "ops".to_owned())],
                    "Rina",
                )
                .await
                .unwrap();
            }
            crate::routes::catalog::prime_search_copy_for_test(
                &state,
                vec![
                    json!({ "id": "silver.a", "name": "Orders" }),
                    json!({ "id": "silver.b", "name": "Customers" }),
                ],
            )
            .await;

            // Governance Admin without `catalog:read`: it may delete a key, but
            // it may not list the tables the sentence would name.
            let writer = Principal {
                permissions: PermissionSet::parse("governance:write"),
                ..admin()
            };
            let (status, refused) = answer(
                delete_tag_key(
                    State(state.clone()),
                    Extension(writer),
                    Path("data_domain".to_owned()),
                    HeaderMap::new(),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            let message = refused["error"].as_str().unwrap();
            assert!(message.contains("2 tables."), "{refused}");
            assert!(!message.contains("Orders"), "{refused}");
            assert!(!message.contains("Customers"), "{refused}");
        }

        // PR #106 review R1 BLOCKER 1
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_caller_outside_the_catalog_tenant_is_refused_every_tag_key_write(
            pool: lakehouse_store::PgPool,
        ) {
            let t1 = Uuid::new_v4();
            let t2 = Uuid::new_v4();
            sqlx::query("INSERT INTO tenant (id, name, slug, plan, residency) VALUES ($1, 'T1', 't1', 'Standard', 'US'), ($2, 'T2', 't2', 'Standard', 'US')")
                .bind(t1)
                .bind(t2)
                .execute(&pool)
                .await
                .unwrap();

            let options = pool.connect_options();
            let mut env = HashMap::new();
            env.insert(
                "DATABASE_URL".to_owned(),
                format!(
                    "postgres://{}:postgres@{}:{}/{}",
                    options.get_username(),
                    options.get_host(),
                    options.get_port(),
                    options.get_database().expect("a named database"),
                ),
            );
            env.insert("CATALOG_TENANT_ID".to_owned(), t1.to_string());
            let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));

            let outside_caller = Principal {
                id: PrincipalId::User(Uuid::new_v4()),
                tenant_ids: vec![t2],
                display_name: "Outsider".to_owned(),
                permissions: PermissionSet::parse("governance:write"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            };

            // PR #106 review R2 SHOULD-FIX 2: list_tag_keys is refused for outside caller
            let (status, resp) = answer(
                list_tag_keys(
                    State(state.clone()),
                    Extension(outside_caller.clone()),
                    HeaderMap::new(),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert!(resp["error"].as_str().unwrap().contains(
                "this deployment's shared catalog is owned by the tenant named in CATALOG_TENANT_ID"
            ));

            let (status, resp) = answer(
                create_tag_key(
                    State(state.clone()),
                    Extension(outside_caller.clone()),
                    HeaderMap::new(),
                    Bytes::from_static(br#"{"key":"outside_key","allowedValues":["v1"]}"#),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert!(resp["error"].as_str().unwrap().contains(
                "this deployment's shared catalog is owned by the tenant named in CATALOG_TENANT_ID"
            ));

            let (status, resp) = answer(
                update_tag_key(
                    State(state.clone()),
                    Extension(outside_caller.clone()),
                    Path("outside_key".to_owned()),
                    HeaderMap::new(),
                    Bytes::from_static(br#"{"allowedValues":["v1","v2"]}"#),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert!(resp["error"].as_str().unwrap().contains(
                "this deployment's shared catalog is owned by the tenant named in CATALOG_TENANT_ID"
            ));

            let (status, resp) = answer(
                delete_tag_key(
                    State(state.clone()),
                    Extension(outside_caller),
                    Path("outside_key".to_owned()),
                    HeaderMap::new(),
                )
                .await,
            )
            .await;
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert!(resp["error"].as_str().unwrap().contains(
                "this deployment's shared catalog is owned by the tenant named in CATALOG_TENANT_ID"
            ));

            let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM governed_tag_key")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(stored, 0);
        }
    }

    // ── DATA-42 D1/D3: the primary registry wins. ────────────────────────
    //
    // The classification list is built from BOTH Bronze registries
    // (`lake.bronze_meta.*`, the primary one, and
    // `lake.bronze_meta_sec.*`, the secondary one). A slug that is in
    // both was listed TWICE, under the same id `c-<slug>` -- once per
    // registry.
    //
    // These tests run against a REAL `ClickHouse` (the compose file's own
    // `26.8` tag, through `testcontainers`), because the fix is a
    // statement: `FINAL` and the `NOT IN (SELECT slug FROM ...)` clause
    // are the engine's, and a fake that answered them would only prove
    // that this module asks for what it means to ask for.
    mod registry_clash {
        #![allow(clippy::unwrap_used, clippy::expect_used)]

        use lakehouse_clickhouse::ChClient;

        use super::*;
        use crate::routes::bronze_registry_fixture::{
            clickhouse, seed_catalog_with_tier, seed_sync_with_author,
        };

        /// The rows of the list that classify `slug`.
        fn classifications_of<'a>(body: &'a Value, slug: &str) -> Vec<&'a Value> {
            let id = Value::String(format!("c-{slug}"));
            body["classifications"]
                .as_array()
                .expect("the list has classifications")
                .iter()
                .filter(|row| row["id"] == id)
                .collect()
        }

        /// A slug the two registries both hold: one dataset, one
        /// classification, and the PRIMARY's table and tier (both rows carry
        /// a level as their tier, so each one would be reported; only the
        /// primary may be).
        #[tokio::test]
        async fn the_classification_list_shows_a_slug_that_is_in_both_registries_once() {
            let (url, _container) = clickhouse().await;
            let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());

            seed_catalog_with_tier(
                &ch,
                "bronze_meta",
                "clash_gov",
                "Primary governance",
                "clash_gov_raw",
                "confidential",
            )
            .await;
            seed_sync_with_author(
                &ch,
                "bronze_meta",
                "clash_gov",
                "Primary governance",
                "clash_gov_raw",
                10,
                "primary_team",
            )
            .await;
            seed_catalog_with_tier(
                &ch,
                "bronze_meta_sec",
                "clash_gov",
                "Secondary governance",
                "clash_gov_sec_raw",
                "restricted",
            )
            .await;
            seed_sync_with_author(
                &ch,
                "bronze_meta_sec",
                "clash_gov",
                "Secondary governance",
                "clash_gov_sec_raw",
                99,
                "secondary_team",
            )
            .await;

            let body = classification(&ch, None).await.unwrap();
            let rows = classifications_of(&body, "clash_gov");
            assert_eq!(rows.len(), 1, "one dataset, one classification: {rows:?}");
            assert_eq!(
                rows[0]["asset"],
                json!("bronze.clash_gov_raw"),
                "the primary's table, not the secondary's: {rows:?}"
            );
            assert_eq!(
                rows[0]["classification"],
                json!("confidential"),
                "the primary's tier, not the secondary's: {rows:?}"
            );
        }

        /// `WEB-3` S3a-D1: a Bronze policy's asset id is its registry slug,
        /// read through the same reader the catalog uses; a table no
        /// registry row names has none, and a Silver table needs no registry
        /// at all.
        #[tokio::test]
        async fn a_bronze_policy_names_the_registry_slug_of_its_table() {
            let (url, _container) = clickhouse().await;
            let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());
            seed_catalog_with_tier(
                &ch,
                "bronze_meta",
                "policy_orders",
                "Policy orders",
                "policy_orders_raw",
                "primer",
            )
            .await;

            let conditions = |table: &str| {
                Some(format!(
                    r#"{{"roles":["Analyst"],"table":"{table}","mask":["email"]}}"#
                ))
            };
            let registered = conditions("bronze.policy_orders_raw");
            let unknown = conditions("bronze.no_such_table");
            let silver = conditions("silver.orders");
            let policies = vec![
                stored_policy("Column mask", registered.as_deref()),
                stored_policy("Column mask", unknown.as_deref()),
                stored_policy("Column mask", silver.as_deref()),
            ];
            let ids = PolicyView::asset_ids(&ch, &policies).await;
            assert_eq!(ids[0], Some("policy_orders".to_owned()), "the slug");
            assert_eq!(ids[1], None, "no registry row names it: no invented page");
            assert_eq!(ids[2], Some("silver.orders".to_owned()), "the table itself");
        }

        /// A slug a second run wrote again is classified once, not twice.
        ///
        /// DATA-42 D3: without `FINAL` the two copies of the row are two
        /// rows, so the list carries the dataset twice.
        #[tokio::test]
        async fn a_slug_written_twice_to_the_primary_is_classified_once() {
            let (url, _container) = clickhouse().await;
            let ch = ChClient::new(url.clone(), "default".to_owned(), String::new());

            // The two copies of the row stay apart: no merge may collapse
            // them before the list reads them.
            ch.exec(
                "SYSTEM STOP MERGES lake.`bronze_meta.dataset_catalog`",
                None,
            )
            .await
            .expect("stop the merges of the catalog table");

            seed_catalog_with_tier(
                &ch,
                "bronze_meta",
                "twice_gov",
                "Twice governance",
                "twice_gov_raw",
                "internal",
            )
            .await;
            // The same run again, before `ClickHouse` merged the parts.
            seed_catalog_with_tier(
                &ch,
                "bronze_meta",
                "twice_gov",
                "Twice governance again",
                "twice_gov_raw",
                "internal",
            )
            .await;

            let body = classification(&ch, None).await.unwrap();
            let rows = classifications_of(&body, "twice_gov");
            assert_eq!(rows.len(), 1, "one dataset, one classification: {rows:?}");
        }
    }
}
