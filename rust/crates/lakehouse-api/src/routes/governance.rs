//! `GET /api/governance/{kind}`, `GET /api/governance/lineage` — quality,
//! audit, classification, residency, and dataset lineage.
//!
//! Ports `src/app/api/governance/[kind]/route.ts` and
//! `src/app/api/governance/lineage/route.ts`. `lineage` is mounted as a
//! dedicated static route (see `routes/mod.rs`) rather than folded into the
//! `{kind}` dispatch, matching Next.js's separate `lineage/route.ts` file —
//! it is never reached by [`Kind::parse`].

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::SqlLiteral;
use lakehouse_store::PgPool;
use lakehouse_store::governance::{
    self, ClassificationRule, CreateClassificationRuleInput, CreatePolicyInput,
    CreateQualityRuleInput, CreateResidencyRuleInput, Policy, QualityRule, ResidencyRule,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::{js_error, str_col};
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
            ApiJson(json!({ "error": format!("kind tak dikenal: {kind}") })),
        )
            .into_response(),
        parsed => match run(&state, parsed).await {
            Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
            // Every branch shares one outer `catch (e)` returning
            // `{ error: String(e) }` at 503.
            Err(err) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": js_error(err) })),
            )
                .into_response(),
        },
    }
}

async fn run(state: &AppState, kind: Kind) -> Result<Value, GovError> {
    match kind {
        Kind::Quality => quality(&state.clickhouse, state.pg.as_deref()).await,
        Kind::Audit => audit(&state.dagster).await,
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
    let rows = ch
        .rows(
            "SELECT tabel, cek, argMax(verdict, dibuat_pada) verdict,
                toString(argMax(nilai, dibuat_pada)) nilai,
                toString(max(dibuat_pada)) at
         FROM _silver_meta.quality GROUP BY tabel, cek ORDER BY tabel, cek LIMIT 500",
            None,
        )
        .await?;
    let mut quality: Vec<Value> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let cek = str_col(r, "cek");
            let tabel = str_col(r, "tabel");
            let verdict = str_col(r, "verdict");
            let at = str_col(r, "at");
            let (name, dimension) = if let Some(col) = cek.strip_prefix("null_rate:") {
                (format!("Konversi kolom {col}"), "validity")
            } else if cek == "row_count" {
                (cek.to_owned(), "completeness")
            } else {
                (cek.to_owned(), "accuracy")
            };
            let threshold = if cek.starts_with("null_rate") {
                "null <5%"
            } else {
                "row_count > 0 & tidak anjlok >50%"
            };
            json!({
                "id": format!("q-{i}"),
                "name": name,
                "asset": tabel,
                "dimension": dimension,
                "threshold": threshold,
                "severity": severity_of(verdict),
                "lastStatus": status_of(verdict),
                "lastRunAt": at,
            })
        })
        .collect();
    if let Some(pg) = pg {
        let authored = governance::list_quality_rules(pg).await?;
        quality.extend(authored.iter().filter_map(|r| serde_json::to_value(r).ok()));
    }
    Ok(json!({ "quality": quality }))
}

async fn audit(dagster: &DgClient) -> Result<Value, GovError> {
    let runs = dagster.list_runs(50).await?;
    let audit: Vec<Value> = runs
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
            })
        })
        .collect();
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
    let rows = ch
        .rows(
            "SELECT table_name, run_at, \
                toString(dry_run_deleted_data_files) dry_data, \
                toString(dry_run_deleted_manifest_files) dry_manifests, \
                toString(applied_deleted_data_files) applied_data, \
                toString(applied_deleted_manifest_files) applied_manifests, \
                skipped_verbs \
             FROM lake.`bronze_meta.maintenance_run` \
             ORDER BY run_at DESC LIMIT 500",
            None,
        )
        .await?;
    let runs: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "tableName": str_col(r, "table_name"),
                "runAt": str_col(r, "run_at"),
                "dryRun": {
                    "deletedDataFiles": str_col(r, "dry_data"),
                    "deletedManifestFiles": str_col(r, "dry_manifests"),
                },
                "applied": {
                    "deletedDataFiles": str_col(r, "applied_data"),
                    "deletedManifestFiles": str_col(r, "applied_manifests"),
                },
                "skippedVerbs": str_col(r, "skipped_verbs"),
            })
        })
        .collect();
    Ok(json!({ "maintenance": runs }))
}

/// `GET /api/governance/replication` — R5's slot-lag/WAL-retention metrics
/// surface. Reads `lake.bronze_meta.replication_slot` directly (Dagster-
/// written only, same posture as `maintenance` — no `pg` union). On a
/// deployment where no CDC connector has ever run, this table does not
/// exist yet and the query fails, surfaced as the standard 503 every other
/// `kind` already gives — not a special case.
async fn replication(ch: &ChClient) -> Result<Value, GovError> {
    let rows = ch
        .rows(
            "SELECT connector_id, slot_name, checked_at, active, \
                toString(wal_retained_bytes) wal_retained_bytes, \
                toString(confirmed_flush_lag_bytes) confirmed_flush_lag_bytes, \
                status \
             FROM lake.`bronze_meta.replication_slot` \
             ORDER BY checked_at DESC LIMIT 500",
            None,
        )
        .await?;
    let slots: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({
                "connectorId": str_col(r, "connector_id"),
                "slotName": str_col(r, "slot_name"),
                "checkedAt": str_col(r, "checked_at"),
                "active": str_col(r, "active") == "1" || str_col(r, "active") == "true",
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

/// `GET /api/governance/lineage?focus=<slug>`.
pub async fn lineage(State(state): State<AppState>, Query(q): Query<LineageQuery>) -> Response {
    match lineage_body(&state.clickhouse, &q.focus).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        // `catch (e) { return NextResponse.json({ error: String(e), focus,
        // nodes: [], edges: [], columnMappings: [] }, { status: 503 }); }`.
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({
                "error": js_error(err),
                "focus": q.focus,
                "nodes": [],
                "edges": [],
                "columnMappings": [],
            })),
        )
            .into_response(),
    }
}

async fn lineage_body(ch: &ChClient, focus: &str) -> Result<Value, ChError> {
    let escaped_focus = SqlLiteral::from(focus);
    let meta_sql = format!(
        "SELECT table_name, title, tier FROM lake.`bronze_meta.dataset_catalog` WHERE slug={escaped_focus}
         UNION ALL SELECT table_name, title, tier FROM lake.`bronze_meta_sec.dataset_catalog` WHERE slug={escaped_focus} LIMIT 1"
    );
    let meta_rows = ch.rows(&meta_sql, None).await?;
    let Some(meta) = meta_rows.first() else {
        return Ok(json!({ "focus": focus, "nodes": [], "edges": [], "columnMappings": [] }));
    };

    let table = str_col(meta, "table_name");
    let sekunder = str_col(meta, "tier") == "sekunder";
    let bronze_ns = if sekunder { "bronze_sec" } else { "bronze_sdi" };

    let escaped_table = SqlLiteral::from(table);
    let cols_sql = format!(
        "SELECT kolom, tipe FROM _silver_meta.kolom_tipe WHERE tabel={escaped_table} LIMIT 200"
    );
    let cols = ch.rows(&cols_sql, None).await?;

    let src_label = if sekunder {
        "Sumber sekunder (olahan)"
    } else {
        "Satu Data Jakarta"
    };
    let nodes = json!([
        { "id": "src", "label": src_label, "kind": "source" },
        { "id": format!("bronze.{table}"), "label": format!("Bronze · {table}"), "kind": "iceberg-table" },
        { "id": format!("silver.{table}"), "label": format!("Silver · {table}"), "kind": "view" },
    ]);
    let edges = json!([
        { "id": "e1", "from": "src", "to": format!("bronze.{table}"), "kind": "pipeline" },
        { "id": "e2", "from": format!("bronze.{table}"), "to": format!("silver.{table}"), "kind": "transform" },
    ]);
    let column_mappings: Vec<Value> = cols
        .iter()
        .map(|c| {
            let kolom = str_col(c, "kolom");
            let tipe = str_col(c, "tipe");
            let transform = match tipe {
                "teks" => "bersih_teks (String)",
                "angka" => "angka_id (Decimal)",
                _ => "tanggal_id (Date)",
            };
            json!({
                "source": format!("{bronze_ns}.{table}.{kolom}"),
                "target": format!("silver.{table}.{kolom}"),
                "transform": transform,
            })
        })
        .collect();

    Ok(json!({
        "focus": focus,
        "nodes": nodes,
        "edges": edges,
        "columnMappings": column_mappings,
    }))
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
    body: Bytes,
) -> Response {
    match Kind::parse(&kind) {
        Kind::Quality => match create_quality_rule(State(state), body).await {
            Ok(resp) => resp.into_response(),
            Err(err) => err.into_response(),
        },
        Kind::Classification => match create_classification_rule(State(state), body).await {
            Ok(resp) => resp.into_response(),
            Err(err) => err.into_response(),
        },
        Kind::Residency => match create_residency_rule(State(state), body).await {
            Ok(resp) => resp.into_response(),
            Err(err) => err.into_response(),
        },
        Kind::Audit | Kind::Maintenance | Kind::Replication | Kind::Unknown => (
            StatusCode::BAD_REQUEST,
            ApiJson(
                json!({ "error": format!("kind tak dikenal atau tidak bisa ditulis: {kind}") }),
            ),
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

/// `POST /api/governance/policies` — author a new policy. Returns 201.
///
/// # Errors
///
/// 400 on a malformed body; 409 if the name is taken; 503/500 as above.
pub async fn create_policy(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<Policy>)> {
    let body: CreatePolicyBody = parse_body(&body)?;
    let input = CreatePolicyInput {
        name: body.name,
        kind: body.kind,
        subjects: body.subjects,
        resources: body.resources,
        effect: body.effect,
        conditions: body.conditions,
        activate: body.activate,
        owner: body.owner,
    };
    let created = governance::create_policy(pool(&state)?, &input).await?;
    Ok((StatusCode::CREATED, ApiJson(created)))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

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
}
