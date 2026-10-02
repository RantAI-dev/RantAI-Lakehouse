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
use axum::http::StatusCode;
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
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::lakehouse::is_unknown_table_error;
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
    /// `governance/classification` — per-asset classification (currently
    /// always `"internal"`).
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
pub async fn get(State(state): State<AppState>, Path(kind): Path<String>) -> Response {
    match Kind::parse(&kind) {
        Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": format!("unknown kind: {kind}") })),
        )
            .into_response(),
        parsed => match run(&state, parsed).await {
            Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
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
    is_unknown_table_error(body)
        || body.contains("(UNKNOWN_DATABASE)")
        || body.contains("Code: 81.")
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
fn actor_kind_of(principal_kind: Option<&str>) -> &'static str {
    match principal_kind {
        Some("user") => "user",
        Some("service") => "service",
        // "copilot" and "schedule" are both non-human actors from the
        // frontend's point of view.
        _ => "agent",
    }
}

/// Same narrowing for `outcome`: `audit_event.outcome` has eight values,
/// the TS `AuditOutcome` union has three (`"success" | "denied" |
/// "error"`). `needs_confirmation`/`needs_approval` are in-flight, not yet
/// allowed, so they map to `"denied"` rather than `"success"` — a viewer
/// scanning for problems should not read a still-pending gate as clean.
fn outcome_of(outcome: &str) -> &'static str {
    match outcome {
        "allowed" | "executed" | "approved" => "success",
        "failed" => "error",
        // "refused" | "rejected" | "needs_confirmation" | "needs_approval"
        _ => "denied",
    }
}

/// `GET /api/governance/audit`. Unions `audit_event` rows (copilot/console
/// actions) onto the `Dagster`-run-derived pipeline history, adding a
/// `source: "copilot" | "pipeline"` field so a consumer can filter by
/// origin. This is additive: every field the pre-existing response shape
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
            json!({
                "id": e.id,
                "at": e.at,
                "actor": e.actor_label.clone().or_else(|| e.principal_id.clone()).unwrap_or_else(|| "Copilot".to_owned()),
                "actorKind": actor_kind_of(e.principal_kind.as_deref()),
                "tenant": TENANT_ID.as_str(),
                "action": e.action,
                "resource": resource,
                "outcome": outcome_of(&e.outcome),
                "policyDecision": if outcome_of(&e.outcome) == "success" { "allow" } else { "deny" },
                "obligations": [],
                "approvalId": e.approval_id,
                "source": "copilot",
            })
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

/// **Gap fix** — same shape as [`quality`], unioning authored classification
/// rules onto the `ClickHouse`-derived per-asset list.
async fn classification(ch: &ChClient, pg: Option<&PgPool>) -> Result<Value, GovError> {
    let rows = ch
        .rows(
            "SELECT slug, title, tier FROM lake.`bronze_meta.dataset_catalog`
         UNION ALL SELECT slug, title, tier FROM lake.`bronze_meta_sec.dataset_catalog` LIMIT 500",
            None,
        )
        .await?;
    let mut classifications: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "id": format!("c-{}", str_col(r, "slug")),
                "asset": str_col(r, "title"),
                "classification": "internal",
                "confidence": 1,
                "reviewStatus": "auto",
            })
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

/// Builds the `SELECT` behind [`latest_maintenance_run`]. `table_name`
/// narrows to one table (`WHERE table_name = {SqlLiteral} ... LIMIT 1`);
/// `None` keeps the all-tables query byte-identical to what
/// `GET /api/governance/maintenance` has always run (`ORDER BY run_at DESC
/// LIMIT 500`, no `WHERE`).
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
/// exist yet and the query fails, surfaced as the standard 503 every other
/// `kind` already gives — not a special case.
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

async fn replication(ch: &ChClient) -> Result<Value, GovError> {
    let rows = ch.rows(REPLICATION_SLOTS_SQL, None).await?;
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
/// 400 for an unrecognized `kind` or a malformed body; 503/500 as above.
pub async fn create_rule(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> Response {
    let actor = principal.as_ref().map(|Extension(p)| p);
    match Kind::parse(&kind) {
        Kind::Quality => match create_quality_rule(State(state.clone()), body).await {
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
        },
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

/// `GET /api/governance/policies` — every authored policy.
///
/// # Errors
///
/// 503 if no pool is configured; 500 on a database failure.
pub async fn list_policies(State(state): State<AppState>) -> ApiResult<ApiJson<Vec<Policy>>> {
    Ok(ApiJson(governance::list_policies(pool(&state)?).await?))
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

/// `POST /api/governance/policies` — author a new policy. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body or an authored `conditions` blob with no real
/// obligation; 409 if the name is taken; 503/500 as above.
pub async fn create_policy(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<Policy>)> {
    let body: Value = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
    let input = create_policy_body(body)?;
    let created = governance::create_policy(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
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
) -> ApiResult<(StatusCode, ApiJson<Policy>)> {
    let created = create_policy(State(state.clone()), body).await?;
    let policy = &created.1.0;
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
/// # Errors
///
/// `400` for a malformed body or any other status; `404` when no policy
/// has that id; 503/500 as above.
pub async fn set_policy_status(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Policy>> {
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
    Ok(ApiJson(policy))
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
            let expanded = crate::sql_rewrite::validate_row_filter_expr(filter, &real_columns)
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
/// 400 on a malformed body; 409 if the name is taken; 503/500 as above.
pub async fn create_quality_rule(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<QualityRule>)> {
    let body: CreateQualityRuleBody = parse_body(&body)?;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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

    // ── WS7 item A5: real policy impact preview ─────────────────────────

    #[test]
    fn preview_body_reports_unmeasured_when_conditions_do_not_parse() {
        let v = preview_result_json(None, None);
        assert!(v["affectedRows"].is_null());
        assert!(v["maskedColumns"].as_array().unwrap().is_empty());
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

        assert!(rows.is_empty());
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
        assert_eq!(actor_kind_of(None), "agent");
    }

    #[test]
    fn outcome_of_treats_pending_gates_as_denied_not_success() {
        assert_eq!(outcome_of("allowed"), "success");
        assert_eq!(outcome_of("executed"), "success");
        assert_eq!(outcome_of("approved"), "success");
        assert_eq!(outcome_of("failed"), "error");
        assert_eq!(outcome_of("refused"), "denied");
        assert_eq!(outcome_of("rejected"), "denied");
        // The regression this test guards: a still-pending gate decision
        // must never read as "success" just because it isn't a hard
        // refusal yet.
        assert_eq!(outcome_of("needs_confirmation"), "denied");
        assert_eq!(outcome_of("needs_approval"), "denied");
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
}
