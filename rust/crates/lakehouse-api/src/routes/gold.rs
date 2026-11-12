//! `POST`/`GET /api/gold/export/{mart}` — ADR 0010's Gold export to
//! Iceberg, triggered over HTTP.
//!
//! `POST` runs the export (read `{gold_source_schema}.{mart}` from
//! `ClickHouse`, append it to the Gold Iceberg table `{mart}` through
//! Lakekeeper — see `crate::gold_export`); `GET` reads the Gold Iceberg
//! table straight back through `iceberg-rust` and reports its row count
//! and format version, independent of whatever `POST` claimed — this is
//! the round-trip proof the acceptance test (`ops/gold_export/`) uses.
//!
//! This is a Rust/Phase-P6-only surface — no `TypeScript` route exists to
//! port, since the original backend never wrote Iceberg. The trigger this
//! build wires up is `dagster/dispar_orchestrate/gold_export.py`, the same
//! "Dagster calls the Rust API over HTTP" shape `routes::pipelines`
//! already uses in reverse (`lakehouse-api` calling Dagster) — here
//! Dagster is the caller.
//!
//! # Auth: same D4 shape as `/api/alerts/run`
//!
//! [`check_export_token`] mirrors `routes::alerts::check_run_token`
//! exactly: with `GOLD_EXPORT_RUN_TOKEN` configured, a matching
//! `x-run-token` header/`?token=` query param is required; with it unset,
//! only a `PrincipalId::Service` principal (the scheduler's own
//! credential) is let through — never a bare `RequiresAuth` pass, and
//! never unauthenticated. `POLICY_TABLE` still requires `RequiresAuth` as
//! the floor (see `crate::policy`), matching `/api/alerts/run`'s own
//! belt-and-suspenders shape.
//!
//! # This route's boot posture: no hard dependency on Lakekeeper being provisioned
//!
//! `docker-compose.yml`'s `lakehouse-api` service does not (any more) wait
//! on `lakekeeper-authz-init` finishing before it starts — this crate's
//! whole documented posture (`lakehouse_store::connect_lazy`,
//! `AppState::pg`'s doc comment) is that the process boots and serves
//! every route it can even when a dependency is down, degrading only the
//! routes that need it. [`export`]/[`read_back`] report the missing
//! precondition at REQUEST time instead: [`read_catalog_token`] returns
//! `503 Unavailable` with fixed text naming the env var (ADR 0011, via the
//! shared `crate::lakekeeper_token::read_token_file` helper — never the
//! token path or the raw io error) when Gold export has not been
//! provisioned on this deployment yet, rather than the whole process
//! refusing to start over one route's dependency.
//!
//! # Single-flight: concurrent exports of the SAME mart
//!
//! [`export`] acquires a per-mart lock (`AppState::gold_export_locks` —
//! `crate::gold_lock`) before doing anything else; a second concurrent
//! call for the SAME mart gets `409 Conflict` immediately rather than
//! racing the first (both would read `ClickHouse` and append to Iceberg,
//! duplicating every row). See [`export`]'s own doc comment for the
//! design tradeoff (409, not queueing).
//!
//! # Content Fingerprint & Cost (DATA-10, D5)
//!
//! `?ifChanged=true` skips export when the `ClickHouse` content fingerprint
//! (row count, wrapping 64-bit sum of `cityHash64(*)`, and schema hash)
//! matches the last successful export — and never when the mart's newest run
//! failed (`DATA-49`, QA `LH-DATA-1-01`). A full scan over a 5,000,000-row mart
//! (`GOLD_EXPORT_MAX_ROWS`) with 4 realistic columns (`UInt64`, `String`, `Float64`,
//! `DateTime`) measured 187 ms (0.187s) on `ClickHouse` 26.8 running on Linux
//! `x86_64` (7.9 GB RAM, 6 vCPUs).

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use lakehouse_core::secret::SecretValue;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::gold_export::{self, GoldExportError};
use crate::gold_export_history;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};

impl From<GoldExportError> for ApiError {
    fn from(err: GoldExportError) -> Self {
        match err {
            GoldExportError::ClickHouse(e) => upstream_error::ch_error(&upstream_error::DATABASE, &e),
            // QA LH-DATA-1-01 (DATA-49): the batch text can hold a cell value
            // and the Arrow or Iceberg library's words, so it is classified
            // like an upstream failure — fixed sentence plus a reference —
            // instead of being stored or answered verbatim.
            GoldExportError::Batch(detail) => upstream_error::report(
                &upstream_error::PUBLISH_BATCH,
                upstream_error::Class::Failed,
                &detail,
            )
            .into_api_error(upstream_error::FailedAs::Unprocessable),
            GoldExportError::UnsupportedColumn { .. }
            // The mart is over `GOLD_EXPORT_MAX_ROWS` — a well-formed
            // request against real (too-large) data, same "caller's input,
            // not our outage" shape as the other `Unprocessable` variants,
            // not a 500: the fix is retrying with a smaller mart or a
            // deliberately raised cap, not anything server-side.
            | GoldExportError::RowCapExceeded { .. }
            // WS7 item D2: `err.to_string()` is safe here — the inner
            // string is already `policy_engine::enforcement_error_message`'s
            // own fixed, non-leaking text (`export_batch_sql` never wraps
            // the raw Postgres/sqlparser error), matching AGENTS.md rule 4.
            | GoldExportError::PolicyRefused(_) => Self::Unprocessable(err.to_string()),
            // GOV-7 D5: a table the owner may not read travels as the answer
            // itself — the tenant gate's own 404 and sentence (D2).
            GoldExportError::PolicyTableRefused(refusal) => refusal,
            // SEC-50: the tenant gate's own error travels as it is, so
            // the caller sees the status the gate would have answered
            // (its fixed sentence, never an upstream text).
            GoldExportError::TenantGateRefused(gate) => gate,
            // SEC-11: the Iceberg writer's text (catalog URLs, object keys)
            // goes to the log under a reference, not into the body.
            GoldExportError::Iceberg(e) => {
                upstream_error::internal_error(&upstream_error::CATALOG, &e)
            }
        }
    }
}

/// Query parameters shared by `POST`/`GET /api/gold/export/{mart}`.
#[derive(Debug, Deserialize, Default)]
pub struct ExportQuery {
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header — same shape as `routes::alerts::RunQuery::token`.
    token: Option<String>,
    /// DATA-1 task 4: `?ifChanged=true` skips the export when the mart
    /// provably has not changed since its last successful export (see
    /// [`if_changed_skip_reason`]). Any other value is treated as absent
    /// (booleans are parsed with `== "true"`), so the console's manual
    /// trigger — which sends nothing — can never accidentally skip.
    #[serde(rename = "ifChanged")]
    if_changed: Option<String>,
}

/// DATA-10 review SHOULD-FIX 2 / DATA-10 / DATA-1: `?ifChanged=true` skip decision.
///
/// Pure function so the outcomes are unit-tested without any server.
/// Skips when the current fingerprint equals the fingerprint on the newest
/// successful export run. Anything unknown (no successful export yet, empty
/// stored fingerprint from a pre-DATA-10 run, no active mart fingerprint)
/// exports: a stale open-format copy is the worse outcome, so uncertainty fails
/// toward publishing.
///
/// `DATA-49` (QA `LH-DATA-1-01`): `newest_run_failed` is a fourth such doubt and
/// wins over a matching fingerprint. The comparison is against the last
/// *successful* run, so after a failure on an unchanged mart every scheduled run
/// would be skipped and the card would say "Last publish failed" until someone
/// pressed Publish now — and a run that failed between batches also leaves a
/// partial newest copy. A failed newest run therefore always publishes.
fn if_changed_skip_reason(
    current_fp: Option<&str>,
    last_fp: Option<&str>,
    last_exported_ms: Option<i64>,
    newest_run_failed: bool,
) -> Option<String> {
    if newest_run_failed {
        return None;
    }
    let current = current_fp?;
    let last = last_fp?;
    if current.is_empty() || last.is_empty() || current != last {
        return None;
    }
    let since = last_exported_ms
        .and_then(millis_to_rfc3339)
        .or_else(|| last_exported_ms.map(|ms| format!("unix ms {ms}")))
        .unwrap_or_else(|| "last export".to_string());
    Some(format!("unchanged since {since}"))
}

/// The skipped-response body `POST` returns when `?ifChanged=true` found
/// nothing to do: `200`, explicitly `skipped`, with the reason the
/// decision produced. Deliberately NOT an export record — see
/// [`export`]'s "Skip" doc section.
fn skipped_body(reason: &str) -> Value {
    json!({ "skipped": true, "reason": reason })
}

/// The freshness reads behind `?ifChanged=true`: `Some(reason)` to skip with,
/// `None` to export. A failed read is a classified [`ApiError`], not a silent
/// skip — a scheduler that cannot MEASURE must not treat the mart as quiet.
///
/// `DATA-49` (QA `LH-DATA-1-01`): the newest run of any status is read first. A
/// failed newest run answers `None` without the successful-run read, because a
/// failure is reason enough to publish (see [`if_changed_skip_reason`]).
async fn unchanged_since_last_export_reason(
    state: &AppState,
    mart: &str,
    current_fingerprint: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let newest = gold_export_history::last_export_run(&state.clickhouse, mart)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    let newest_failed = newest.as_ref().is_some_and(|run| run.status == "failed");
    if newest_failed {
        return Ok(None);
    }
    let last_export = gold_export_history::last_success_export(&state.clickhouse, mart)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    let last_fp = last_export.as_ref().map(|e| e.content_fingerprint.as_str());
    let last_started_ms = last_export.as_ref().and_then(|e| e.started_at_ms);
    Ok(if_changed_skip_reason(
        current_fingerprint,
        last_fp,
        last_started_ms,
        newest_failed,
    ))
}

/// See the module doc comment's "Auth" section. Two independent ways to
/// pass, checked in this order:
///
/// 1. **`gold:export` permission, unconditionally.** A principal (human or
///    service) holding this permission — e.g. Platform Admin's seeded
///    `*:*` role, or a narrower role an operator has explicitly granted it
///    — passes regardless of whether `GOLD_EXPORT_RUN_TOKEN` is
///    configured. This is what lets the console's "Export now" button
///    (WS6) work in every deployment, including ones that configure a
///    shared token for the Dagster schedule — D4 (`CHANGELOG.md`:
///    "`/api/alerts/run` now fails closed (401) when `ALERTS_RUN_TOKEN` is
///    unset, instead of allowing unauthenticated calls") forbids
///    fail-open when a token is unset; it does not require that a
///    configured token become the only accepted credential once one
///    exists — an explicitly permissioned session is exactly the kind of
///    caller this floor exists to let through, not turn away.
/// 2. **The configured shared token, if `GOLD_EXPORT_RUN_TOKEN` is set.**
///    Unchanged from before this permission fallback existed: a wrong or
///    missing token here is always `401`, whether or not the caller holds
///    any permission at all.
///
/// 3. **QA LH-DATA-1-02 (plan D6): a signed-in person who presented no
///    token is `403`, not `401`.** In the configured-token branch, a
///    `PrincipalId::User` without `gold:export` that sent no token (header
///    and query both absent) is forbidden: they are signed in, and the
///    console reads every `401` as an expired session, which sent an Analyst
///    opening a Gold table page into a `/login` loop. A token that is
///    presented and wrong stays `401`. Nobody gets in who did not before.
///
/// With neither set, only a `PrincipalId::Service` principal is let
/// through (`503` otherwise) — unchanged from before this task.
fn check_export_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(p) = principal
        && p.has("gold:export")
    {
        return Ok(());
    }
    if let Some(need) = configured {
        let presented = header_token.or(query_token);
        return if presented == Some(need) {
            Ok(())
        } else if presented.is_none()
            && matches!(principal, Some(p) if matches!(p.id, PrincipalId::User(_)))
        {
            // QA LH-DATA-1-02 (plan D6): signed in, no token, no gold:export.
            // `PermissionDenied`, not `Forbidden`: the latter's fixed body is
            // `embedding_disabled`, which would mislabel this 403.
            Err(ApiError::PermissionDenied("gold:export".to_owned()))
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        // Unchanged: the scheduled Dagster trigger's own service identity
        // always passes even with no explicit scope, same posture as
        // before this task.
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "gold export is not configured: set GOLD_EXPORT_RUN_TOKEN, call with a service \
             identity credential, or use a session with the gold:export permission"
                .to_owned(),
        )),
    }
}

/// Reads the `gold-export` principal's Lakekeeper bearer token from
/// [`crate::config::Config::lakekeeper_gold_export_token_file`].
///
/// Thin, Gold-specific wrapper over the shared
/// [`crate::lakekeeper_token::read_token_file`] helper (WS2 task A0
/// generalized this function rather than copying it for the new
/// `lakehouse-api-reader` principal — AGENTS.md rule 4). No token refresh:
/// the file is re-read on every call, so a re-minted token is picked up on
/// the next call, but nothing proactively re-mints one before its 30-day
/// expiry (ADR 0011's known gap).
///
/// # Errors
///
/// Returns [`ApiError::Unavailable`] with fixed text naming only the
/// purpose and `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` if the file cannot be
/// read (Gold export is not provisioned on this deployment — see ADR
/// 0011: the `gold-export` principal's token is minted by `ops/oidc-mock`
/// at compose bring-up onto a volume this service must have mounted). The
/// path and the io error are logged, never returned to the caller.
///
/// `pub(crate)` (not private) so `routes::ai::tools::gold` (T2.4 of the
/// copilot-operations-handover plan) can read the SAME token this route
/// reads, rather than a second copy of this file-read — see that module's
/// doc comment for why the copilot tool calls this directly instead of
/// going through [`export`]/[`read_back`]'s [`check_export_token`] guard.
pub(crate) async fn read_catalog_token(path: &str) -> Result<SecretValue, ApiError> {
    crate::lakekeeper_token::read_token_file(
        path,
        "gold-export",
        "LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE",
    )
    .await
}

/// `SEC-50` R4: whose reach the Gold export is measured against.
///
/// A signed-in PERSON reads the shared catalog as themselves: their own
/// tenant must own it, exactly as in Query Studio. A service identity and
/// the shared run token are jobs with no user, and they act as the tenant
/// that owns the shared catalog (`SEC-50` D3).
///
/// One place, so the route and the copilot's tool pick the same actor
/// (`AGENTS.md` rule 4).
pub(crate) fn gold_export_gate_actor<'a>(
    principal: Option<&'a Principal>,
    headers: &'a HeaderMap,
) -> crate::routes::query::GateActor<'a> {
    match principal {
        // A job with no user: the scheduled export, the run token.
        None => crate::routes::query::GateActor::CatalogTenant,
        // A service identity: a job with no user either.
        Some(principal) if matches!(principal.id, PrincipalId::Service(_)) => {
            crate::routes::query::GateActor::CatalogTenant
        }
        // A signed-in person: their own tenant, the `X-Tenant` header
        // narrowing it exactly as it does everywhere else.
        Some(principal) => crate::routes::query::GateActor::Caller { principal, headers },
    }
}

/// `SEC-50`: the name the export reads the mart under, and the tenant
/// gate on it.
///
/// The export reads the SHARED catalog, so the gate asks whose reach
/// covers it: a signed-in person's own tenant, or the catalog's own
/// tenant for a job with no user (`SEC-50` R4, D3). A deployment that
/// holds several tenants and names no catalog tenant has no tenant whose
/// reach covers the shared catalog, and the export is refused — before the
/// fingerprint, which is the export's first read.
///
/// `export_mart` asks the same gate for the rows it reads, so a caller
/// that reaches the export through another path (the copilot's tool) is
/// gated there too.
///
/// # Errors
///
/// The gate's own errors: 422 with the fixed sentence of
/// [`crate::routes::query::JOB_NEEDS_CATALOG_TENANT`] for a deployment
/// with several tenants and no catalog tenant; 404 with the gate's table
/// sentence for a person whose tenant does not own the shared catalog; 503
/// with the shared catalog's own sentence for a deployment that cannot say
/// how many tenants it holds (`SEC-50` R5, amended: a job with no user in
/// a deployment with no database to count in).
async fn gated_source_table(
    state: &AppState,
    mart: &str,
    actor: crate::routes::query::GateActor<'_>,
) -> Result<String, ApiError> {
    let source_table = format!("{}.`{}`", state.config.gold_source_schema, mart);
    crate::routes::query::gate_statement(
        state,
        &gold_export::mart_count_sql(&source_table),
        &sqlparser::dialect::ClickHouseDialect {},
        actor,
    )
    .await?;
    Ok(source_table)
}

/// `POST /api/gold/export/{mart}` — run the export.
///
/// # Skip: `?ifChanged=true` (DATA-1 task 4, DATA-10)
///
/// DATA-10 review SHOULD-FIX 2: With that parameter, a mart whose current
/// content fingerprint (`count()`, wrapping 64-bit sum of `cityHash64(*)`,
/// and column schema hash) matches the fingerprint stored on its last
/// successful export is skipped: `200` with `{"skipped": true,
/// "reason": "unchanged since <rfc3339>"}`, and NOTHING else happens —
/// no export, no `console.gold_export_run` row, no Iceberg snapshot.
/// Unlike the earlier part-modification timestamp check, the content
/// fingerprint does not false-export on table rewrites that leave row
/// content identical (such as `OPTIMIZE TABLE FINAL` or `EXCHANGE TABLES`).
/// This is what the scheduler sends so a quiet mart costs reading the
/// last export history and one fingerprint query. Unknown facts never
/// skip (see [`if_changed_skip_reason`]); the console's manual trigger
/// does not send the parameter at all. A mart whose newest run FAILED is
/// never skipped, even when its fingerprint matches the last success: the
/// card would otherwise say "Last publish failed" until someone pressed
/// Publish now, and a run that failed between batches leaves a partial
/// newest copy (`DATA-49`, QA `LH-DATA-1-01`).
///
/// # Single-flight: only one export of a given mart runs at a time
///
/// A per-mart lock (`AppState::gold_export_locks` — see
/// `crate::gold_lock`'s module doc comment) is acquired before anything
/// else happens. If a second call for the SAME mart arrives while the
/// first is still running, this returns `409 Conflict` immediately rather
/// than letting both calls read `ClickHouse` and append to Iceberg — two
/// concurrent exports of the same mart would each append the mart's
/// current rows, duplicating them in the target table (`GoldTable::append`
/// has no dedup). A DIFFERENT mart is unaffected — the lock is keyed by
/// mart name.
///
/// # What is recorded (`DATA-56`, #484 review R4)
///
/// The history table is the card's source of truth, so a publish that fails
/// has to leave a run there — otherwise a mart whose every publish fails before
/// the copy starts stays "Up to date" or "Out of date" for ever (#484 review R4,
/// `DATA-56`). After the caller is authenticated and allowed and the mart
/// is shown to be theirs, every failure is recorded: the content fingerprint
/// read, the `?ifChanged=true` history read, the catalog token file, and the
/// export itself. A refusal records nothing: `401`/`403`/`503` from
/// [`check_export_token`], `400` for a mart name that is not an identifier, the
/// tenant gate's `404`/`422`/`503`, and the `409` of a publish already in
/// flight (that publish records its own run, and a row for the turned-away call
/// would be newer and hide its result). A name that is not a table is not
/// refused — it goes on and fails where it fails today (D23). The row's
/// sentence is the sentence the response carries ([`stored_error`]), never
/// upstream text. When the database that keeps the history is the thing that
/// failed, nothing can be recorded: the write is best-effort and the caller
/// gets the failure it already had.
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`], `409` if an export of the
/// SAME mart is already in flight, 422 with the tenant gate's fixed
/// sentence when the deployment holds several tenants and names no
/// catalog tenant (`SEC-50`), 404 with the gate's own table sentence for
/// a signed-in person whose tenant does not own the shared catalog
/// (`SEC-50`; PR #281 review round 2 NIT), 503 if the Lakekeeper token
/// cannot be read, 422 for an unsupported source column, a source mart
/// over `GOLD_EXPORT_MAX_ROWS`, or a bad `ClickHouse` query, or 500 for
/// an Iceberg catalog/write failure.
pub async fn export(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    // SEC-50: the export reads the SHARED catalog, so the gate asks
    // whose reach covers it — a signed-in person's own tenant, or the
    // catalog's own tenant for a job with no user (R4, D3). The gate runs
    // BEFORE the first read of the mart — the fingerprint below is that
    // read — so a deployment that holds several tenants and names no
    // catalog tenant refuses the export before anything reaches
    // `ClickHouse`.
    let actor = gold_export_gate_actor(principal.as_ref().map(|Extension(p)| p), &headers);
    let source_table = gated_source_table(&state, mart_ident.as_str(), actor).await?;

    let triggered_by = triggered_by(principal.as_ref().map(|Extension(p)| p));

    // #484 review R4 (DATA-56): from here on the caller is
    // authenticated and allowed and the mart is shown to be theirs, so every
    // failure below is a failed publish and is recorded as one.
    let attempt = Attempt::open(&state, mart_ident.as_str(), &triggered_by).await?;

    // DATA-10 review SHOULD-FIX 1 / DATA-10 D2: The fingerprint is computed before
    // the export reads rows, and written on the history row of that export.
    let current_fingerprint =
        read_content_fingerprint(&attempt, &state.config.gold_source_schema).await?;

    // DATA-10 / DATA-1 task 4 — `?ifChanged=true`: a proven-unchanged mart is
    // skipped BEFORE the single-flight lock is taken (a skip holds
    // nothing, writes no `console.gold_export_run` row, and commits no
    // snapshot — the response is the only trace). Unknown facts export, per
    // [`if_changed_skip_reason`].
    if query.if_changed.as_deref() == Some("true")
        && let Some(body) = attempt.skip_body(current_fingerprint.as_deref()).await?
    {
        return Ok(ApiJson(body));
    }

    // Held for the rest of this handler (dropped at function return,
    // success or error alike) — see this function's "Single-flight" doc
    // section above for why acquiring it is the very first thing done
    // after validating the mart name. A `409` here records NO run (DATA-56):
    // the publish that holds the lock records its own row, and a row written
    // by the call it turned away would be newer and hide that result.
    let _export_guard = state
        .gold_export_locks
        .try_acquire(mart_ident.as_str())
        .await
        .map_err(|_| {
            ApiError::Conflict(format!(
                "export for mart {:?} is already running; wait for it to finish before retrying",
                mart_ident.as_str()
            ))
        })?;

    let token = attempt
        .catalog_token(current_fingerprint.as_deref())
        .await?;
    let iceberg_config = gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );

    // WS7 item D2: `export_mart`'s per-batch query goes through the same
    // `sql_rewrite::enforce` Query Studio/dashboards use. `principal`
    // being `None` here means the call authenticated purely via the
    // `x-run-token`/`?token=` shared secret (`check_export_token` above,
    // matching `GOLD_EXPORT_RUN_TOKEN` with no principal at all) — a
    // genuine service-identity trigger with no role membership, mapped to
    // `roles: &[]`/`PlaceholderValues::none()` (a `PolicyCondition` can
    // never target an empty-`roles` subject, WS7 item A3, so this never
    // silently resolves to "no obligations" for a REAL principal — see
    // this route's own doc comment on `check_export_token`'s three
    // distinct authentication shapes). A `Some(principal)` (human session
    // holding `gold:export`, or the scheduler's own `Service` principal)
    // always uses that principal's real `role_names`/id/tenant ids.
    let roles: Vec<String> = principal
        .as_ref()
        .map_or_else(Vec::new, |Extension(p)| p.role_names.clone());
    let placeholders = principal.as_ref().map_or_else(
        crate::sql_rewrite::PlaceholderValues::none,
        |Extension(p)| crate::sql_rewrite::PlaceholderValues {
            principal_id: Some(p.id.uuid().to_string()),
            principal_tenant_ids: p.tenant_ids.iter().map(ToString::to_string).collect(),
        },
    );
    let obligations = crate::policy_engine::PolicyEngineObligations::from_state(&state);

    let started_at = time::OffsetDateTime::now_utc();
    let export_result = gold_export::export_mart(
        &state,
        actor,
        &state.clickhouse,
        &iceberg_config,
        &source_table,
        mart_ident.as_str(),
        state.config.gold_export_max_rows,
        state.config.gold_export_batch_size,
        &roles,
        &placeholders,
        &obligations,
    )
    .await;
    let finished_at = time::OffsetDateTime::now_utc();
    // QA LH-DATA-1-01 (DATA-49): one conversion, before the row is stored, so
    // the history row and the response carry the same classified sentence and
    // the same reference. `GoldExportError`'s `Display` is the log's only.
    let export_result = export_result.map_err(ApiError::from);

    record_export_history(
        &state,
        mart_ident.as_str(),
        &triggered_by,
        started_at,
        finished_at,
        current_fingerprint.as_deref().unwrap_or(""),
        export_result.as_ref(),
    )
    .await;

    let result = export_result?;

    Ok(ApiJson(json!({
        "namespace": result.namespace,
        "table": result.table,
        "formatVersion": result.format_version,
        "rowsExported": result.rows_exported,
        "snapshotId": result.snapshot_id,
        "exportedAt": result.exported_at_ms.and_then(millis_to_rfc3339),
    })))
}

/// Best-effort history write for one finished export attempt: every
/// `POST` that gets past the mart's existence records exactly one row, success
/// or failure (the acceptance test counts on it), but a failed write is
/// logged, never propagated — this table is a history view, not a correctness
/// dependency of the export itself (`gold_export_history::record_export_run`'s
/// doc comment).
///
/// `export_result` is the converted result, so the row's `error` is the
/// sentence the response carries (`DATA-49`, QA `LH-DATA-1-01`) — the
/// `ApiError`'s own text, never the `Display` of a `GoldExportError`, which
/// holds upstream text for the log. A failure that happens before the export
/// starts is recorded from the error alone ([`early_failure`], `DATA-56`,
/// #484 review R4).
#[allow(
    clippy::cast_possible_truncation,
    reason = "rows_exported is a usize from an in-memory Vec of one ClickHouse batch, \
              never near u64::MAX"
)]
async fn record_export_history(
    state: &AppState,
    mart: &str,
    triggered_by: &str,
    started_at: time::OffsetDateTime,
    finished_at: time::OffsetDateTime,
    content_fingerprint: &str,
    export_result: Result<&gold_export::GoldExportResult, &ApiError>,
) {
    let error_message = stored_error(export_result);
    let history_row = gold_export_history::NewGoldExportRun {
        mart,
        status: if export_result.is_ok() {
            "success"
        } else {
            "failed"
        },
        rows_exported: export_result.ok().map(|r| r.rows_exported as u64),
        format_version: export_result.ok().map(|r| r.format_version),
        snapshot_id: export_result.ok().and_then(|r| r.snapshot_id),
        error: error_message.as_deref(),
        triggered_by,
        content_fingerprint,
        started_at_ms: started_at.unix_timestamp() * 1000,
        finished_at_ms: finished_at.unix_timestamp() * 1000,
    };
    if let Err(err) = gold_export_history::record_export_run(&state.clickhouse, &history_row).await
    {
        tracing::error!(%err, mart, "failed to record gold export history");
    }
}

/// Who triggered the publish, for the history row: a service and a person are
/// told apart, and a call that authenticated only through the shared run token
/// names no person at all.
fn triggered_by(principal: Option<&Principal>) -> String {
    principal.map_or_else(
        || "unknown".to_owned(),
        |p| match p.id {
            PrincipalId::Service(_) => format!("service:{}", p.display_name),
            PrincipalId::User(_) => format!("user:{}", p.display_name),
        },
    )
}

/// A publish attempt that has passed the token check, the mart name and the
/// tenant gate — the facts a failure before the export still needs to leave a
/// run with (`DATA-56`, #484 review R4). It carries the gate's answer instead of
/// asking again, so a recording helper cannot be reached for a mart the caller
/// was never shown.
struct Attempt<'a> {
    state: &'a AppState,
    /// Whether `system.tables` lists the mart. The one answer that is never
    /// recorded with a failed run (see [`early_failure`]).
    exists: bool,
    mart: &'a str,
    triggered_by: &'a str,
    /// When this attempt's row starts: the moment the mart was found to exist.
    started_at: time::OffsetDateTime,
}

impl<'a> Attempt<'a> {
    /// Opens the attempt on a mart the caller has already reached: asks whether
    /// `system.tables` lists it, which is the last step that answers without
    /// recording anything (`DATA-56` D19) — the engine that failed holds the
    /// history — and starts the clock its row would carry.
    ///
    /// # Errors
    ///
    /// Returns the classified `ClickHouse` failure of that read; nothing is
    /// recorded for it.
    async fn open(
        state: &'a AppState,
        mart: &'a str,
        triggered_by: &'a str,
    ) -> Result<Self, ApiError> {
        let exists = mart_exists(&state.clickhouse, &state.config.gold_source_schema, mart)
            .await
            .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
        Ok(Self {
            state,
            exists,
            mart,
            triggered_by,
            started_at: time::OffsetDateTime::now_utc(),
        })
    }

    /// `?ifChanged=true`: the skip body when the mart is proven unchanged since
    /// its last successful export, `None` when it must be exported (unknown
    /// facts export, per [`if_changed_skip_reason`]). A read that fails is a
    /// failed publish, not a skip (`DATA-56`, #484 review R4).
    ///
    /// # Errors
    ///
    /// Returns the classified history-read failure, after [`early_failure`] has
    /// recorded it for a mart that exists.
    async fn skip_body(&self, fingerprint: Option<&str>) -> Result<Option<Value>, ApiError> {
        match unchanged_since_last_export_reason(self.state, self.mart, fingerprint).await {
            Ok(Some(reason)) => Ok(Some(skipped_body(&reason))),
            Ok(None) => Ok(None),
            Err(err) => Err(early_failure(self, fingerprint.unwrap_or(""), err).await),
        }
    }

    /// The catalog token the export publishes with. A token that cannot be read
    /// is a failed publish, recorded before the error is returned (`DATA-56`).
    ///
    /// # Errors
    ///
    /// Returns [`lakekeeper_token::token_unavailable_message`]'s fixed sentence
    /// as a `503`, after it is recorded for a mart that exists.
    async fn catalog_token(
        &self,
        fingerprint: Option<&str>,
    ) -> Result<lakehouse_core::secret::SecretValue, ApiError> {
        match read_catalog_token(&self.state.config.lakekeeper_gold_export_token_file).await {
            Ok(token) => Ok(token),
            Err(err) => Err(early_failure(self, fingerprint.unwrap_or(""), err).await),
        }
    }
}

/// The mart's content fingerprint, or `None` for a name that is not a table
/// (`DATA-10` D2). A read that fails is a failed publish, recorded before the
/// error is returned (`DATA-56`).
///
/// # Errors
///
/// Returns the classified `ClickHouse` failure (`422`/`503` with a reference),
/// after [`early_failure`] has recorded it for a mart that exists.
async fn read_content_fingerprint(
    attempt: &Attempt<'_>,
    schema: &str,
) -> Result<Option<String>, ApiError> {
    if !attempt.exists {
        // DATA-56 D23: a name that is not a table goes on as it did before —
        // it is not refused here and no early failure is recorded for it.
        return Ok(None);
    }
    match gold_export_history::mart_content_fingerprint(
        &attempt.state.clickhouse,
        schema,
        attempt.mart,
    )
    .await
    {
        Ok(fingerprint) => Ok(fingerprint),
        Err(e) => {
            let err = upstream_error::ch_error(&upstream_error::DATABASE, &e);
            Err(early_failure(attempt, "", err).await)
        }
    }
}

/// Records a publish that failed before `export_mart` was reached, and returns
/// the error the caller answers with (`DATA-56`, #484 review R4).
///
/// The rule (`DATA-56` D19): once the caller is authenticated and allowed and
/// the mart is shown to be theirs, a failure is a failed publish and leaves a
/// run — the fingerprint read, the `?ifChanged=true` history read, and the
/// catalog token are all inside that boundary. Before it, nothing is recorded:
/// the token check, the mart name, and the tenant gate all refuse the request
/// rather than fail it. `attempt.exists` carries the one answer that may not be
/// recorded: the read of `system.tables` that failed — nothing is known about
/// the mart then, and the engine that failed holds the history.
///
/// The write is best-effort, like every other history write here: when
/// `ClickHouse` is what failed, the row cannot be written and the card's own
/// read fails with it, so nothing can be said about "Last publish failed"
/// (`DATA-56` D21). The caller's error is returned unchanged either way.
async fn early_failure(
    attempt: &Attempt<'_>,
    content_fingerprint: &str,
    err: ApiError,
) -> ApiError {
    if attempt.exists {
        record_export_history(
            attempt.state,
            attempt.mart,
            attempt.triggered_by,
            attempt.started_at,
            time::OffsetDateTime::now_utc(),
            content_fingerprint,
            Err(&err),
        )
        .await;
    }
    err
}

/// The sentence a failed publish stores, or `None` for a success.
///
/// Takes the converted result, not the `GoldExportError`, so this module cannot
/// store an upstream text by accident (`DATA-49`, QA `LH-DATA-1-01`): the
/// `ApiError` is what the response carries, and the row and the response are
/// meant to read the same. Borrowed, not owned, so a failure recorded before
/// the export starts can be stored and then answered (`DATA-56`).
fn stored_error(result: Result<&gold_export::GoldExportResult, &ApiError>) -> Option<String> {
    result.err().map(ToString::to_string)
}

/// Renders an `Iceberg` snapshot's Unix-millisecond commit time as an RFC
/// 3339 UTC string, or `None` if `ms` doesn't fall on a valid instant
/// (defensive only — every value this is ever called with comes from
/// `Snapshot::timestamp_ms`, which `iceberg-rust` derives from the
/// table's own committed metadata).
fn millis_to_rfc3339(ms: i64) -> Option<String> {
    time::OffsetDateTime::from_unix_timestamp(ms / 1000)
        .ok()
        .and_then(|dt| {
            dt.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
}

/// `GET /api/gold/export/{mart}` — read the Gold Iceberg table back
/// through `iceberg-rust` and report its row count, independent of
/// whatever the last `POST` claimed. This is the round-trip proof; it
/// does not touch `ClickHouse` at all.
///
/// A mart the catalog does not have answers `200` with `published: false`
/// and every other fact `null` ([`read_back_body`]) — never published is a
/// state, not an error (`DATA-49`, QA `LH-DATA-1-01`).
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`] (same guard as `POST` —
/// this still reveals row counts, not public data), 503 if the Lakekeeper
/// token cannot be read, or 500 if the catalog refuses or fails the call
/// for any reason other than "table not found", or the Iceberg read fails.
pub async fn read_back(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    let token = read_catalog_token(&state.config.lakekeeper_gold_export_token_file).await?;
    let iceberg_config = gold_export::iceberg_config(
        state.config.lakekeeper_catalog_uri.clone(),
        state.config.lakekeeper_warehouse.clone(),
        Some(token),
    );

    let readback = gold_export::read_back_row_count(&iceberg_config, mart_ident.as_str())
        .await
        .map_err(ApiError::from)?;

    Ok(ApiJson(read_back_body(
        mart_ident.as_str(),
        readback.as_ref(),
    )))
}

/// The body of `GET /api/gold/export/{mart}` and the assistant's
/// `get_gold_export` tool, in one place so the two cannot drift (`DATA-49`,
/// QA `LH-DATA-1-01`).
///
/// A mart never published answers `200` with `published: false` and every
/// other fact `null`, never an error and never an invented number: the
/// console treats "no export" as a normal state, and a 404 would make it a
/// failed request in the browser for something that is not an error.
pub(crate) fn read_back_body(mart: &str, readback: Option<&gold_export::GoldReadBack>) -> Value {
    match readback {
        None => json!({
            "namespace": lakehouse_iceberg::gold::GOLD_NAMESPACE,
            "table": mart,
            "published": false,
            "formatVersion": Value::Null,
            "rowsInIceberg": Value::Null,
            "snapshotId": Value::Null,
            "exportedAt": Value::Null,
        }),
        Some(readback) => json!({
            "namespace": lakehouse_iceberg::gold::GOLD_NAMESPACE,
            "table": mart,
            "published": true,
            "formatVersion": readback.format_version,
            "rowsInIceberg": readback.rows,
            "snapshotId": readback.snapshot_id,
            "exportedAt": readback.exported_at_ms.and_then(millis_to_rfc3339),
        }),
    }
}

/// Query parameters for `GET /api/gold/exports`.
#[derive(Debug, Deserialize)]
pub struct ExportsQuery {
    mart: String,
    /// Same shared-token/permission gate as `export`/`read_back` — see
    /// [`check_export_token`].
    token: Option<String>,
}

/// `GET /api/gold/exports?mart=` — the most recent export attempts for one
/// mart, newest first, from `console.gold_export_run`
/// (`gold_export_history`). Same auth posture as `read_back`: it reveals
/// row counts and timing, not public data.
///
/// # Errors
///
/// Returns 400 if `mart` is missing/blank or not a valid identifier,
/// 401/503 from [`check_export_token`], or 500 if the `ClickHouse` query
/// fails.
pub async fn exports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ExportsQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let mart_ident = Ident::new(query.mart.trim())
        .map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    let runs = gold_export_history::list_export_runs(&state.clickhouse, mart_ident.as_str(), 50)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;

    Ok(ApiJson(
        json!({ "mart": mart_ident.as_str(), "runs": runs }),
    ))
}

/// `GET /api/gold/export/{mart}/consumers` — who read this mart's
/// published table, and how often (DATA-5, ADR 0016, plan D7).
///
/// The count is the API's own: every table load through the door
/// (`routes::iceberg_door`) is counted, and this route reads those
/// counts back. It answers:
///
/// - when outside reading is set up on this deployment:
///   `{ mart, supported: true, windowDays: 7, consumers: [{ identityId,
///   name, reads, lastReadAt }] }`, with the reads summed over the last
///   seven calendar days and the list ordered by reads descending;
/// - when it is not: `{ mart, supported: false, consumers: null, reason:
///   <the door's own reason> }` — the SAME sentence the door's own 503
///   carries, so the console says exactly what a tool would be told.
///   Never a count of zero: "not measured" and "measured, and nobody
///   read it" are different answers (principle 2).
///
/// The mart is looked up under the name the EXPORT gave its table in the
/// `gold` namespace (`lakehouse_iceberg::gold::sanitize_mart_name`), so
/// the count is found under exactly that name.
///
/// # Errors
///
/// Returns 400 if `mart` is not a valid identifier; 503 when this
/// deployment has no Postgres pool; 500 when the count cannot be read.
pub async fn consumers(
    State(state): State<AppState>,
    Path(mart): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;
    let mart_name = mart_ident.as_str();

    // The door's own readiness decides whether the count is measured at
    // all (D4, D7). Its refusal text is the door's own, so the console
    // and a tool are told the same thing.
    if let Err(problem) = crate::routes::iceberg_door::setup(&state).await {
        return Ok(ApiJson(json!({
            "mart": mart_name,
            "supported": false,
            "consumers": Value::Null,
            "reason": problem.reason(),
        })));
    }

    // The published table's name in the `gold` namespace — the export
    // names it this way (`gold::sanitize_mart_name`), so the count is
    // looked up under exactly that name.
    let table = lakehouse_iceberg::gold::sanitize_mart_name(mart_name)
        .map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;

    let pool = pool(&state)?;
    // Seven calendar days, today included: the window the page shows.
    let since = time::OffsetDateTime::now_utc().date() - time::Duration::days(WINDOW_DAYS - 1);
    let consumers = lakehouse_store::outside_reads::outside_readers_of(
        pool,
        crate::routes::iceberg_door::GOLD_NAMESPACE,
        &table,
        since,
    )
    .await
    .map_err(ApiError::from)?;

    Ok(ApiJson(json!({
        "mart": mart_name,
        "supported": true,
        "windowDays": WINDOW_DAYS,
        "consumers": consumers,
    })))
}

/// How many days of reads the consumers route reports (DATA-5's spec:
/// "the page shows the last 7 days"; the history itself is kept for 90).
const WINDOW_DAYS: i64 = 7;

// ── Per-mart publication switches (DATA-1) ──────────────────────────────
//
// Whether `serving.{mart}` is being kept fresh in the `gold` Iceberg
// namespace is a per-mart setting (`gold_publication`, migration 0054)
// instead of the Build-menu button ADR 0010 shipped first: the scheduler
// reads the enabled set every night, an authored pipeline run re-exports
// its own mart, and switching a mart off only flips the flag — the
// Iceberg copy is never dropped by this feature (DATA-1 decision table).

/// Whether `{schema}.{mart}` exists in `system.tables` — the PUT route's
/// 404 gate. Same `system.tables` read `routes::catalog` builds its Gold
/// asset list from; the identifier is `Ident`-validated before it is
/// ever interpolated.
///
/// # Errors
///
/// Returns [`ChError`] if the query fails (a dead `ClickHouse` surfaces
/// as its classified [`ApiError`], never raw text).
// DATA-10 review SHOULD-FIX 1: delegating to gold_export_history::mart_exists
async fn mart_exists(ch: &ChClient, schema: &str, mart: &str) -> Result<bool, ChError> {
    gold_export_history::mart_exists(ch, schema, mart).await
}

/// Unix-millisecond `max(modification_time)` over the active parts of
/// `{schema}.{mart}`, or `None` when it has no active parts (a view, or
/// an engine without parts) — "not measured", never a guessed time.
///
/// DATA-10 review SHOULD-FIX 2: Measured, not assumed, on the compose
/// file's own `ClickHouse` 26.8 (plan §6): every INSERT creates a part
/// whose `modification_time` is the write time. This timestamp is still
/// queried for `lastChangedAt` in [`publication_body`] for console
/// display, but it is no longer used for any export or skip decision
/// (DATA-10 replaced timestamp-based skip with content fingerprinting).
/// The count guard is what makes a parts-less table `None` rather than
/// the epoch.
///
/// # Errors
///
/// Returns [`ChError`] if the query fails.
async fn mart_last_changed_at(
    ch: &ChClient,
    schema: &str,
    mart: &str,
) -> Result<Option<i64>, ChError> {
    let sql = format!(
        "SELECT toString(count()) AS parts, \
                toString(toUnixTimestamp(max(modification_time))) AS changed_s \
         FROM system.parts \
         WHERE database = {schema} AND table = {mart} AND active",
        schema = lakehouse_core::ident::SqlLiteral::from(schema),
        mart = lakehouse_core::ident::SqlLiteral::from(mart),
    );
    let rows = ch.rows(&sql, None).await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let parts = row.get("parts").and_then(Value::as_str).unwrap_or("0");
    if parts == "0" {
        return Ok(None);
    }
    Ok(row
        .get("changed_s")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .map(|s| s * 1000))
}

/// The response body shared by the publication detail `GET` and `PUT`:
/// the stored switch plus the two measured freshness facts the console's
/// "Up to date"/"Out of date" line compares.
/// DATA-10 D4: Compare the current mart fingerprint with the last recorded
/// successful export's fingerprint.
///
/// Returns:
/// - `Some(false)` when both fingerprints are non-empty and identical (Up to date).
/// - `Some(true)` when both fingerprints are non-empty and differ (Out of date).
/// - `None` when either fingerprint is missing or empty (Not measured / Never published).
#[must_use]
pub fn compute_changed_since_export(
    current_fp: Option<&str>,
    last_fp: Option<&str>,
) -> Option<bool> {
    let current = current_fp?;
    let last = last_fp?;
    if current.is_empty() || last.is_empty() {
        return None;
    }
    Some(current != last)
}

/// The response body shared by the publication detail `GET` and `PUT`:
/// the stored switch plus the freshness facts the console compares.
///
/// `SEC-50` slice 2 T8 (M12): `changedSinceExport` is a READ of the mart —
/// the content fingerprint counts its rows — so the body asks the tenant
/// gate for the route's caller first, through the same helper the export
/// uses ([`gated_source_table`]), before its first `ClickHouse` read. A
/// caller whose reach does not cover the shared catalog gets the gate's
/// 404 and the mart is never read.
async fn publication_body(
    state: &AppState,
    mart: &str,
    enabled: bool,
    updated_at: Option<String>,
    can_edit: bool,
    actor: crate::routes::query::GateActor<'_>,
) -> Result<Value, ApiError> {
    // `SEC-50` slice 2 T8 (M12): the gate is the export's own, so the two
    // paths cannot drift. Its answer is the table name the export reads;
    // this body reads through the fingerprint helpers, which take the
    // schema and the mart, so only the refusal is used here.
    gated_source_table(state, mart, actor).await?;
    let last_changed_at =
        mart_last_changed_at(&state.clickhouse, &state.config.gold_source_schema, mart)
            .await
            .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    let last_export = gold_export_history::last_success_export(&state.clickhouse, mart)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    // DATA-10 review SHOULD-FIX 1: mart_content_fingerprint moved to gold_export_history
    let current_fingerprint = gold_export_history::mart_content_fingerprint(
        &state.clickhouse,
        &state.config.gold_source_schema,
        mart,
    )
    .await
    .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    let changed_since_export = compute_changed_since_export(
        current_fingerprint.as_deref(),
        last_export.as_ref().map(|e| e.content_fingerprint.as_str()),
    );
    let last_exported_at = last_export.as_ref().and_then(|e| e.started_at_ms);
    // `DATA-49` (QA `LH-DATA-1-01`): the newest run of any status, so the card
    // can say the last publish failed instead of reading "Up to date" through
    // five failed runs. A failed read here is classified like its neighbours.
    let newest_run = gold_export_history::last_export_run(&state.clickhouse, mart)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    // PR #105 removed From<ChError>; classify, never surface ClickHouse text.
    let last_trim = gold_export_history::last_trim(&state.clickhouse, mart)
        .await
        .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    Ok(json!({
        "mart": mart,
        "enabled": enabled,
        "updatedAt": updated_at,
        "lastChangedAt": last_changed_at.and_then(millis_to_rfc3339),
        "lastExportedAt": last_exported_at.and_then(millis_to_rfc3339),
        "changedSinceExport": changed_since_export,
        "lastFailure": last_failure(newest_run.as_ref()),
        "canEdit": can_edit,
        "trim": last_trim,
    }))
}

/// The `lastFailure` field of [`publication_body`] (`DATA-49`, QA
/// `LH-DATA-1-01`): `null` unless the newest run of the mart is `failed`, then
/// the time it started and the sentence that was stored for it.
///
/// `lastExportedAt` and `changedSinceExport` keep describing the last
/// *successful* publish, so the card can show both facts at once. A row with no
/// stored reason says so instead of inventing one.
fn last_failure(newest: Option<&gold_export_history::LastExportRun>) -> Value {
    let Some(run) = newest.filter(|run| run.status == "failed") else {
        return Value::Null;
    };
    json!({
        "failedAt": run.started_at_ms.and_then(millis_to_rfc3339),
        "reason": run
            .error
            .clone()
            .unwrap_or_else(|| "The publish failed. No reason was recorded.".to_owned()),
    })
}

/// Borrow the Postgres pool, or fail with a 503. Mirrors
/// `routes::identity::pool`/`routes::pipelines::pool`: an UNCONFIGURED
/// Postgres is `Unavailable`, not `Internal` — the deployment is missing
/// `DATABASE_URL`, which is a "this route cannot work here" state, the
/// repo's documented 503 (PR slice B review S1).
fn pool(state: &AppState) -> Result<&sqlx::PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "gold publications unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// `GET /api/gold/publications` — the enabled marts, and nothing else.
/// This is the scheduler's source of truth (DATA-1 task 5): a fresh
/// deployment publishes exactly this list, which is empty until a
/// `gold:export` principal switches a mart on.
///
/// The floor is `RequiresAuth` (see `POLICY_TABLE`) PLUS
/// [`check_export_token`] in the handler, same two-layer shape as the
/// export routes: the scheduler authenticates with its run token or
/// service identity; a human session needs `gold:export`. A publication
/// list is not secret the way row data is, but it does reveal what a
/// deployment considers its "open format" copy — same posture as
/// `GET /api/gold/exports`.
///
/// # Errors
///
/// Returns 401/503 from [`check_export_token`], 503 when Postgres is not
/// configured ([`pool`]; this feature is Postgres-backed, unlike the
/// export routes), or 500 for a store failure (classified, never raw
/// upstream text).
pub async fn publications(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ExportQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_export_token(
        state.config.gold_export_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    let pool = pool(&state)?;
    let enabled = lakehouse_store::gold_publication::list_enabled(pool)
        .await
        .map_err(ApiError::from)?;
    let publications: Vec<Value> = enabled
        .iter()
        .map(|p| json!({ "mart": p.mart, "enabled": p.enabled, "updatedAt": p.updated_at }))
        .collect();
    Ok(ApiJson(json!({ "publications": publications })))
}

/// `GET /api/gold/export/{mart}/publication` — one mart's publication
/// state: the stored switch, the two measured freshness facts, and
/// `lastFailure` when the mart's newest run failed (`DATA-49`, QA
/// `LH-DATA-1-01`).
///
/// A mart with no `gold_publication` row is OFF (the default), reported
/// as `enabled: false, updatedAt: null` — a missing row is a state, not
/// an error. `lastChangedAt`/`lastExportedAt` are honest `null`s when
/// they cannot be measured (no active parts; no successful export yet) —
/// the console renders "Not measured"/"Never published" from exactly
/// these nulls, never a guessed time.
///
/// # Errors
///
/// Returns 400 for a mart that is not a valid identifier, 503 when
/// Postgres is not configured ([`pool`]), 404 with the tenant gate's own
/// table sentence for a caller whose reach does not cover the shared
/// catalog (`SEC-50` slice 2 T8), or classified store/`ClickHouse` errors
/// otherwise.
pub async fn publication(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;
    let pool = pool(&state)?;
    let stored = lakehouse_store::gold_publication::get(pool, mart_ident.as_str())
        .await
        .map_err(ApiError::from)?;
    let (enabled, updated_at) = stored.map_or((false, None), |p| (p.enabled, Some(p.updated_at)));
    let can_edit = principal
        .as_ref()
        .is_some_and(|Extension(p)| p.has("gold:export"));
    // `SEC-50` slice 2 T8 (M12/`SEC-50` R4): the body reads the mart, so it
    // is gated as the route's caller — a person as themselves (`X-Tenant`
    // honoured), a job with no user as the catalog's tenant.
    let actor = gold_export_gate_actor(principal.as_ref().map(|Extension(p)| p), &headers);
    let body = publication_body(
        &state,
        mart_ident.as_str(),
        enabled,
        updated_at,
        can_edit,
        actor,
    )
    .await?;
    Ok(ApiJson(body))
}

/// Body of `PUT /api/gold/export/{mart}/publication`.
#[derive(Debug, Deserialize)]
pub struct PublicationBody {
    /// Whether the scheduler should keep this mart's Iceberg copy fresh.
    /// `false` only flips the flag — the existing Iceberg table is never
    /// dropped by this feature (DATA-1 decision table).
    enabled: bool,
}

/// Best-effort audit row for a successful toggle, following
/// `routes::pipelines::record_pipeline_audit`'s posture: a failed audit
/// write is logged, never propagated — it must not turn an already-
/// succeeded configuration change into a 500.
async fn record_publication_audit(
    state: &AppState,
    principal: &Principal,
    mart: &str,
    enabled: bool,
) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let event = NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "gold.publication_set".to_owned(),
        resource_kind: Some("gold_mart".to_owned()),
        resource_id: Some(mart.to_owned()),
        args: Some(json!({ "enabled": enabled })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    };
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::error!(%err, mart, "failed to record gold publication audit");
    }
}

/// `PUT /api/gold/export/{mart}/publication` — switch one mart's
/// publishing on or off. `POLICY_TABLE` floors this route at
/// `RequiresPermission("gold:export")`, so the handler never re-checks
/// the permission (fail closed at the gate); it needs the principal only
/// to record who flipped the switch.
///
/// Returns 404 when `serving.{mart}` does not exist — measured against
/// `system.tables`, not assumed — so a typo cannot silently enable
/// publishing for a mart nobody can build.
///
/// # Errors
///
/// Returns 400 for a mart that is not a valid identifier, 404 for an
/// unknown mart, 503 when Postgres is not configured ([`pool`]), 401 if
/// the gate somehow let a principal-less request through (never, through
/// the router), or classified store/`ClickHouse` errors otherwise.
pub async fn set_publication(
    State(state): State<AppState>,
    Path(mart): Path<String>,
    headers: HeaderMap,
    principal: Option<Extension<Principal>>,
    // The body extractor is deliberately LAST: axum requires consuming
    // extractors at the end of the argument list (every sibling handler
    // here follows the same order).
    Json(body): Json<PublicationBody>,
) -> ApiResult<ApiJson<Value>> {
    let Some(Extension(principal)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let mart_ident =
        Ident::new(&mart).map_err(|e| ApiError::BadRequest(format!("invalid mart: {e}")))?;
    let pool = pool(&state)?;
    let exists = mart_exists(
        &state.clickhouse,
        &state.config.gold_source_schema,
        mart_ident.as_str(),
    )
    .await
    .map_err(|e| upstream_error::ch_error(&upstream_error::DATABASE, &e))?;
    if !exists {
        return Err(ApiError::NotFound(format!(
            "mart {:?} does not exist in {}",
            mart_ident.as_str(),
            state.config.gold_source_schema
        ))
        .into());
    }

    let updated = lakehouse_store::gold_publication::upsert(
        pool,
        mart_ident.as_str(),
        body.enabled,
        principal.id.uuid(),
    )
    .await
    .map_err(ApiError::from)?;
    record_publication_audit(&state, &principal, mart_ident.as_str(), body.enabled).await;

    let body = publication_body(
        &state,
        mart_ident.as_str(),
        updated.enabled,
        Some(updated.updated_at),
        true,
        // `SEC-50` slice 2 T8: the response body reads the mart, so it is
        // gated as the person who flipped the switch (the handler holds
        // their principal; `SEC-50` R4).
        gold_export_gate_actor(Some(&principal), &headers),
    )
    .await?;
    Ok(ApiJson(body))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::http::{Request, StatusCode};
    use lakehouse_auth::PermissionSet;
    use tower::ServiceExt;

    use super::*;
    use crate::config::Config;

    /// Same idiom `routes::connectors::tests::state_without_pool` uses: a
    /// deliberately-malformed `DATABASE_URL` so `AppState::new` boots with
    /// no live Postgres pool, proving a route is reachable (mounted +
    /// policy-gated) without needing a real database.
    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).unwrap())
    }

    /// The consumers route is registered and policy-gated: with no
    /// credential the auth gate answers before the handler body, so this
    /// proves routing + policy wiring (never a 404, which would mean the
    /// route was not registered). What it answers for an AUTHENTICATED
    /// caller needs a Postgres pool, so it is proven in
    /// `tests/iceberg_door.rs` instead.
    #[tokio::test]
    async fn the_gold_consumers_route_is_registered_and_gated() {
        let app = crate::routes::router(state_without_pool());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/gold/export/sales/consumers")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // No principal/token presented -> the router's auth_gate for
        // RequiresAuth still runs before the handler body, so this proves
        // routing + policy wiring (never 404), matching every other
        // "route is registered" test in this crate's style (see
        // routes::connectors::tests, the other "route is registered" tests).
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    fn service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(uuid::Uuid::new_v4()),
            tenant_ids: vec![],
            display_name: "gold-export-scheduler".to_owned(),
            permissions: PermissionSet::parse("gold:export"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn human_principal_with(permissions: &str) -> Principal {
        Principal {
            id: PrincipalId::User(uuid::Uuid::new_v4()),
            tenant_ids: vec![],
            display_name: "operator".to_owned(),
            permissions: PermissionSet::parse(permissions),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn gold_export_permission_passes_even_with_no_token_configured() {
        assert!(
            check_export_token(None, None, None, Some(&human_principal_with("gold:export")))
                .is_ok()
        );
    }

    #[test]
    fn gold_export_permission_passes_even_when_a_token_is_configured_and_not_presented() {
        // D4 (CHANGELOG.md) forbids FAIL-OPEN — letting an unauthenticated
        // or under-permissioned caller through when a token is unset. It
        // does not require that a token, once configured, become the
        // ONLY accepted credential: a principal explicitly granted
        // gold:export (e.g. via a console session) is exactly the kind
        // of caller D4's floor already exists to distinguish from "any
        // signed-up user" — see check_export_token's doc comment. Without
        // this, a deployment that sets GOLD_EXPORT_RUN_TOKEN (for the
        // Dagster schedule) would make the console's "Export now" button
        // permanently unusable for every human operator.
        assert!(
            check_export_token(
                Some("secret"),
                None,
                None,
                Some(&human_principal_with("gold:export")),
            )
            .is_ok()
        );
    }

    #[test]
    fn no_configured_token_still_accepts_a_service_principal_with_no_explicit_scope() {
        // Unchanged behavior from before this task: a service principal
        // was always sufficient, regardless of its own permission set.
        let mut service = service_principal();
        service.permissions = PermissionSet::parse("");
        assert!(check_export_token(None, None, None, Some(&service)).is_ok());
    }

    #[test]
    fn no_configured_token_rejects_a_human_principal_without_the_permission() {
        let err = check_export_token(
            None,
            None,
            None,
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert!(matches!(err, ApiError::Unavailable(_)));
    }

    #[test]
    fn no_configured_token_rejects_no_principal_at_all() {
        assert!(check_export_token(None, None, None, None).is_err());
    }

    #[test]
    fn configured_token_rejects_an_unpermissioned_session_presenting_the_wrong_token() {
        // The token path's refusal is unchanged for anyone NOT holding
        // gold:export: a wrong/missing token still 401s exactly as
        // before this task.
        let err = check_export_token(
            Some("secret"),
            None,
            Some("wrong"),
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert!(
            matches!(err, ApiError::Unauthorized(_)) || err.to_string().contains("unauthorized")
        );
    }

    #[test]
    fn configured_token_forbids_a_signed_in_user_without_the_permission_who_sent_no_token() {
        // QA LH-DATA-1-02 (plan D6): 403 so the console does not read it as
        // an expired session.
        let err = check_export_token(
            Some("secret"),
            None,
            None,
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert!(matches!(err, ApiError::PermissionDenied(_)));
        assert_eq!(err.status(), 403);
    }

    #[test]
    fn configured_token_still_401s_a_user_without_the_permission_who_sent_a_wrong_token() {
        let err = check_export_token(
            Some("secret"),
            Some("wrong"),
            None,
            Some(&human_principal_with("catalog:read")),
        )
        .unwrap_err();
        assert_eq!(err.status(), 401);
    }

    #[test]
    fn configured_token_still_401s_when_there_is_no_principal_and_no_token() {
        let err = check_export_token(Some("secret"), None, None, None).unwrap_err();
        assert_eq!(err.status(), 401);
    }

    // ── compute_changed_since_export (DATA-10 T3 / D4) ──────────────────

    #[test]
    fn compute_changed_since_export_reports_false_on_match() {
        let changed = compute_changed_since_export(Some("10:111:222"), Some("10:111:222"));
        assert_eq!(changed, Some(false));
    }

    #[test]
    fn compute_changed_since_export_reports_true_on_difference() {
        let changed = compute_changed_since_export(Some("10:111:222"), Some("10:333:222"));
        assert_eq!(changed, Some(true));
    }

    #[test]
    fn compute_changed_since_export_reports_none_when_unmeasured() {
        assert_eq!(compute_changed_since_export(None, Some("10:111:222")), None);
        assert_eq!(compute_changed_since_export(Some("10:111:222"), None), None);
        assert_eq!(
            compute_changed_since_export(Some(""), Some("10:111:222")),
            None
        );
        assert_eq!(
            compute_changed_since_export(Some("10:111:222"), Some("")),
            None
        );
        assert_eq!(compute_changed_since_export(None, None), None);
    }

    // ── if_changed_skip_reason (DATA-10 T2 / DATA-1 task 4) ─────────────

    #[test]
    fn unchanged_mart_matching_fingerprint_is_skipped_with_a_reason() {
        let skip = if_changed_skip_reason(
            Some("10:11129132029957349367:16812361877011020269"),
            Some("10:11129132029957349367:16812361877011020269"),
            Some(1_700_000_000_000),
            false,
        )
        .expect("matching fingerprint must skip");
        assert!(skip.starts_with("unchanged since "), "{skip}");
    }

    #[test]
    fn differing_fingerprints_export_fail_toward_publishing() {
        assert!(
            if_changed_skip_reason(
                Some("10:111:222"),
                Some("10:333:222"),
                Some(1_700_000_000_000),
                false,
            )
            .is_none()
        );
    }

    #[test]
    fn no_success_row_exports() {
        assert!(if_changed_skip_reason(Some("10:111:222"), None, None, false).is_none());
    }

    #[test]
    fn empty_stored_fingerprint_exports() {
        assert!(
            if_changed_skip_reason(Some("10:111:222"), Some(""), Some(1_700_000_000_000), false,)
                .is_none()
        );
    }

    #[test]
    fn empty_or_missing_current_fingerprint_exports() {
        assert!(
            if_changed_skip_reason(Some(""), Some("10:111:222"), Some(1_700_000_000_000), false,)
                .is_none()
        );
        assert!(
            if_changed_skip_reason(None, Some("10:111:222"), Some(1_700_000_000_000), false)
                .is_none()
        );
        assert!(if_changed_skip_reason(None, None, None, false).is_none());
    }

    /// QA `LH-DATA-1-01` (`DATA-49`): a failed newest run publishes even when
    /// the fingerprint matches the last success — otherwise every scheduled run
    /// would be skipped and the card would stay "Last publish failed".
    #[test]
    fn a_failed_newest_run_is_never_skipped_even_when_the_fingerprint_matches() {
        assert!(
            if_changed_skip_reason(
                Some("10:111:222"),
                Some("10:111:222"),
                Some(1_700_000_000_000),
                true,
            )
            .is_none(),
            "a failed newest run is a doubt, and doubt fails toward publishing"
        );
    }

    /// The `lastFailure` field is null unless the newest run failed, whatever the
    /// last successful publish says.
    #[test]
    fn last_failure_is_null_when_the_newest_run_succeeded_or_there_is_none() {
        assert_eq!(last_failure(None), Value::Null);
        assert_eq!(
            last_failure(Some(&gold_export_history::LastExportRun {
                status: "success".to_owned(),
                started_at_ms: Some(1_700_000_000_000),
                error: None,
            })),
            Value::Null
        );
    }

    /// A failed newest run carries the time it started and the stored sentence.
    #[test]
    fn last_failure_carries_the_time_and_the_stored_sentence_when_the_newest_run_failed() {
        let failure = last_failure(Some(&gold_export_history::LastExportRun {
            status: "failed".to_owned(),
            started_at_ms: Some(1_700_000_000_000),
            error: Some("The catalog request failed. Reference: abc".to_owned()),
        }));
        assert_eq!(
            failure["reason"],
            "The catalog request failed. Reference: abc"
        );
        assert_eq!(failure["failedAt"], "2023-11-14T22:13:20Z");
    }

    // ── The read-back body (DATA-49, QA LH-DATA-1-01) ─────────────────────

    /// A mart the catalog does not have is "not published": every other fact is
    /// null, so the card invents neither a row count nor a time.
    #[test]
    fn the_read_back_of_a_mart_never_published_says_so_and_invents_no_number() {
        let body = read_back_body("never_published", None);
        assert_eq!(body["published"], Value::Bool(false));
        assert_eq!(body["table"], "never_published");
        assert_eq!(body["namespace"], lakehouse_iceberg::gold::GOLD_NAMESPACE);
        for field in ["formatVersion", "rowsInIceberg", "snapshotId", "exportedAt"] {
            assert!(body[field].is_null(), "{field} must be null: {body}");
        }
    }

    /// A published mart keeps today's fields and says it is published.
    #[test]
    fn the_read_back_of_a_published_mart_keeps_its_fields_and_says_published() {
        let readback = gold_export::GoldReadBack {
            format_version: 2,
            rows: 5,
            snapshot_id: Some(42),
            exported_at_ms: Some(1_700_000_000_000),
        };
        let body = read_back_body("orders_rollup", Some(&readback));
        assert_eq!(body["published"], Value::Bool(true));
        assert_eq!(body["formatVersion"], 2);
        assert_eq!(body["rowsInIceberg"], 5);
        assert_eq!(body["snapshotId"], 42);
        assert_eq!(body["exportedAt"], "2023-11-14T22:13:20Z");
    }

    // ── The stored and answered sentence (DATA-49, QA LH-DATA-1-01) ──────

    /// QA `LH-DATA-1-01` (`DATA-49`): the batch text can hold a cell value and
    /// the Arrow or Iceberg library's words. It is classified — fixed sentence,
    /// reference id — before it reaches a response or a stored row, and the
    /// status stays `422` as it was.
    #[test]
    fn a_batch_failure_answers_a_fixed_sentence_with_a_reference_and_never_the_value() {
        let err = ApiError::from(GoldExportError::Batch(
            "amount: \"12,5\" is not a valid decimal".to_owned(),
        ));
        assert_eq!(err.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let text = err.to_string();
        assert!(
            text.starts_with(
                "A value in this table could not be written in open format. Reference: "
            ),
            "{text}"
        );
        assert!(
            !text.contains("12,5"),
            "the cell value must not travel: {text}"
        );
    }

    /// QA `LH-DATA-1-01` (`DATA-49`): the history row stores the sentence the
    /// response carried, so the card's reason and the answer read the same and
    /// neither holds the engine's own text or its version.
    #[test]
    fn the_stored_error_of_a_failed_publish_is_the_classified_sentence() {
        let result: Result<gold_export::GoldExportResult, ApiError> = Err(ApiError::from(
            GoldExportError::ClickHouse(ChError::Server(
                "Code: 60. DB::Exception: Unknown table (version 26.8.1.1)".to_owned(),
            )),
        ));
        let stored = stored_error(result.as_ref()).expect("a failed publish stores a sentence");
        assert!(
            stored.starts_with("The database request failed. Reference: "),
            "{stored}"
        );
        assert!(!stored.contains("Code: 60"), "{stored}");
        assert!(!stored.contains("26.8"), "{stored}");
        assert!(
            stored_error(Ok(&gold_export::GoldExportResult {
                namespace: "gold".to_owned(),
                table: "orders_rollup".to_owned(),
                format_version: 2,
                rows_exported: 1,
                snapshot_id: Some(1),
                exported_at_ms: Some(1_700_000_000_000),
            }))
            .is_none()
        );
    }

    /// `SEC-50` slice 2 T8 (M12): the publication body reads the mart (its
    /// content fingerprint counts the rows), so it is gated as the route's
    /// caller. A member of a tenant that does not own the shared catalog gets
    /// `SEC-20`'s 404, and the engine is asked nothing at all — not the
    /// fingerprint, not the parts.
    #[sqlx::test(migrations = "../../migrations")]
    async fn another_tenants_member_cannot_read_a_publication(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        use lakehouse_auth::PermissionSet;
        use uuid::Uuid;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        /// The tenant that owns the shared catalog in this deployment.
        const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";
        /// Another tenant of the same deployment.
        const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [], "rows": 0
            })))
            .mount(&server)
            .await;
        let options = pool.connect_options();
        let url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        );
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), url);
        env.insert("CH_URL".to_owned(), server.uri());
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
        let state = AppState::new(Config::from_map(&env).unwrap());

        let person = Principal {
            id: PrincipalId::User(Uuid::from_u128(11)),
            tenant_ids: vec![OTHER_TENANT.parse().expect("a seeded tenant id")],
            display_name: "Other tenant".to_owned(),
            permissions: PermissionSet::parse("gold:export"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        let result = publication(
            State(state),
            Path("orders_rollup".to_owned()),
            HeaderMap::new(),
            Some(Extension(person)),
        )
        .await;

        let rejection = result.expect_err("the shared catalog is not this tenant's to read");
        assert_eq!(
            rejection.0.status(),
            StatusCode::NOT_FOUND,
            "SEC-20's 404, not a 403: {rejection:?}"
        );
        assert_eq!(
            rejection.0.to_string(),
            "Table not found or not available to your organisation: serving.orders_rollup"
        );
        assert!(
            server
                .received_requests()
                .await
                .expect("mock records requests")
                .is_empty(),
            "the refusal precedes the engine: no fingerprint and no parts read"
        );
        Ok(())
    }

    #[tokio::test]
    async fn unchanged_since_last_export_reason_fails_closed_on_read_error() {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        env.insert("CLICKHOUSE_URL".to_owned(), "http://127.0.0.1:1".to_owned());
        let state = AppState::new(Config::from_map(&env).unwrap());
        let result = unchanged_since_last_export_reason(&state, "orders", Some("10:1:2")).await;
        assert!(
            result.is_err(),
            "ClickHouse failure must surface as an error, never a silent skip"
        );
        assert!(
            // SEC-11: an unreachable engine is a 503 through `ch_error`, not
            // the 422 the removed `From<ChError> for ApiError` produced.
            matches!(result.unwrap_err(), ApiError::Unavailable(_)),
            "an unreachable ClickHouse is classified as Unavailable"
        );
    }
}
