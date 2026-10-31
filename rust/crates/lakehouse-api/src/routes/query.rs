//! `POST /api/query/run`, `POST /api/query/estimate` — the ad hoc `SQL`
//! Query Studio.
//!
//! Ports `src/app/api/query/run/route.ts` and
//! `src/app/api/query/estimate/route.ts`.

use std::collections::HashMap;
use std::time::Instant;

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::audit::NewAuditEvent;
use lakehouse_store::queries::{
    self, HistoryFilter, QueryHistoryItem, RecordHistoryInput, SavedQuery,
};
use lakehouse_store::uploads::TableClaim;
use lakehouse_trino::{TrinoError, TrinoResult};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::sql_guard::{is_read_only, is_trino_safe};
use crate::state::AppState;
use crate::upstream_error;

/// The `{sql}` request body both routes accept.
#[derive(Debug, Deserialize)]
struct SqlBody {
    #[serde(default)]
    sql: Option<String>,
    /// Which engine should run this statement: `"clickhouse"` (the
    /// default, when absent) or `"trino"` (WS2 §4). Only [`run`] reads
    /// this — `estimate` always runs `EXPLAIN ESTIMATE` against
    /// `ClickHouse`, unaffected.
    #[serde(default)]
    engine: Option<String>,
}

/// Parse the raw request body as `{"sql": "..."}`.
///
/// Both routes share one `try { ({ sql } = await req.json()) } catch { ...
/// "Body must be JSON {sql}" ... }` shape in the `TypeScript`: any body that
/// doesn't parse as JSON at all — not merely a body missing `sql` — is a
/// 400 with this exact message (byte-for-byte parity corpus match, not
/// merely English-language style).
fn parse_body(body: &Bytes) -> Result<SqlBody, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("Body must be JSON {sql}".to_owned()))
}

/// WS7 item C2: prefetches obligations for every table `sql` references
/// (WS7 item C1's `PolicyEngineObligations`) and rewrites `sql` through
/// [`crate::sql_rewrite::enforce`] — masking/row-filtering every governed
/// table, refusing a table function, a sensitive `system.*` read, a
/// `dictGet`/`joinGet`-family call, or a view over a governed table
/// outright. `sql` is never forwarded to `ClickHouse`/`Trino` unrewritten:
/// every failure here is an `Err`, matching `sql_rewrite`'s own "never a
/// silent pass-through" rule (Hard Requirement 2), and Hard Requirement 1
/// (a `status = 'ready'` policy row with an authored-but-unparseable
/// condition) surfaces the same way — a 422, never a silently-unenforced
/// query — via [`crate::policy_engine::PolicyEngineError::UnenforceableAuthoredCondition`].
///
/// The elapsed wall time is recorded into `state.policy_decision_latencies`
/// (WS7 item C2 Step 4's `policyDecisionP95Ms`) regardless of outcome — a
/// refused query still cost real time deciding that.
///
/// # Errors
/// 422 [`ApiError::Unprocessable`] with a fixed, non-leaking message
/// (`policy_engine::engine_error_message`/`refusal_message` — the
/// underlying Postgres/`sqlparser` error text never reaches the response,
/// `AGENTS.md` rule 4) when obligations cannot be resolved or `sql` fails
/// classification/substitution, and 404 [`ApiError::NotFound`] with the
/// fixed sentence of [`table_not_available`] when the statement reads a
/// table the caller may not read (`SEC-20`), and 422
/// [`ApiError::Unprocessable`] with the fixed sentence of
/// [`LISTING_NOT_AVAILABLE`] when the statement lists objects without
/// naming a table and the caller may not list them (`SEC-20`).
pub(crate) async fn rewrite_sql_for_principal(
    state: &AppState,
    sql: &str,
    engine: &str,
    principal: &Principal,
) -> Result<String, ApiError> {
    rewrite_sql_for_principal_with_headers(state, sql, engine, principal, &HeaderMap::new()).await
}

/// The shared rewrite, with the request's headers.
///
/// `SEC-20`: the tenant the caller acts as comes from the request's
/// `X-Tenant` header when the caller has the request's headers, and from
/// the caller's first tenant when it does not — the chat, the catalog's
/// own sample and profile and the dependents list have no headers to
/// pass. The header only ever narrows the caller's own tenants
/// ([`crate::tenant_scope`]), so it can never widen anything.
///
/// # Errors
///
/// The same as [`rewrite_sql_for_principal`].
pub(crate) async fn rewrite_sql_for_principal_with_headers(
    state: &AppState,
    sql: &str,
    engine: &str,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let started_rewrite = Instant::now();
    let result = rewrite_sql_for_principal_inner(state, sql, engine, principal, headers).await;
    state
        .policy_decision_latencies
        .record(started_rewrite.elapsed());
    result
}

/// `Trino`'s catalog.schema.table qualification needs `GenericDialect` (no
/// `PrestoDialect` exists in this `sqlparser` version — verified, see
/// `sql_rewrite`'s own `generic_dialect_parses_a_representative_trino_shape`
/// test); every other engine value (including the `"clickhouse"` default)
/// uses `ClickHouseDialect`, matching `run`'s own engine dispatch.
fn is_trino_engine(engine: &str) -> bool {
    engine == "trino"
}

async fn rewrite_sql_for_principal_inner(
    state: &AppState,
    sql: &str,
    engine: &str,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations_source = crate::policy_engine::PolicyEngineObligations::from_state(state);

    // SEC-20/SEC-50: the tenant gate runs BEFORE the policy rewrite, so a
    // statement that reads a table the caller may not read is refused
    // before the policy's own prefetch asks the engine about that table,
    // and before the statement reaches the engine at all.
    //
    // The gate's targets come from ONE parse, with the dialect of the
    // engine the statement names (the same choice the policy rewrite
    // below makes): the tables the statement names, or the fact that it
    // lists objects without naming one. A statement that does not parse
    // has no targets at all: the policy rewrite refuses it with a 422 of
    // its own, so there is nothing here to gate.
    let actor = GateActor::Caller { principal, headers };
    if is_trino_engine(engine) {
        gate_statement(state, sql, &sqlparser::dialect::GenericDialect {}, actor).await?;
    } else {
        gate_statement(state, sql, &sqlparser::dialect::ClickHouseDialect {}, actor).await?;
    }

    // WS7 item D1/D2: the actual referenced-tables/prefetch/enforce
    // sequence now lives in ONE place —
    // `policy_engine::rewrite_sql_for_roles` — shared with
    // `routes::support::run_spec_sql` (dashboards/embeds) and
    // `gold_export::export_batch_sql` (Gold export), never a parallel
    // copy of this logic. Only the per-engine dialect *selection*
    // (`ClickHouseDialect` vs. `GenericDialect`) stays here, since the
    // two are different concrete types.
    if is_trino_engine(engine) {
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::GenericDialect {},
            &principal.role_names,
            &placeholders,
            &obligations_source,
        )
        .await
    } else {
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::ClickHouseDialect {},
            &principal.role_names,
            &placeholders,
            &obligations_source,
        )
        .await
    }
    .map_err(|err| {
        // DATA-16 F3: on Trino a past-version clause makes the parse fail
        // (`GenericDialect` reads no table version), and the generic
        // answer talks about policies. Say what is actually wrong. This
        // runs only after a parse failure, so a query that parses is
        // never affected, and it is a message, not a gate: the query was
        // already refused.
        if is_trino_engine(engine)
            && matches!(
                err,
                crate::policy_engine::EnforcementError::Engine(
                    crate::policy_engine::PolicyEngineError::Unparseable
                )
            )
            && uses_past_version_clause(sql)
        {
            return ApiError::Unprocessable(PAST_VERSION_TRINO_MESSAGE.to_owned());
        }
        ApiError::Unprocessable(crate::policy_engine::enforcement_error_message(&err).to_owned())
    })
}

/// `SEC-50`: who a gated statement acts as.
///
/// The gate is ONE rule with three actors, not three rules: every site
/// that runs a statement through the policy rewrite asks this gate
/// first, and the actor says whose reach the statement is measured
/// against. Nobody writes a second copy of the reach logic
/// (`AGENTS.md` rule 4).
#[derive(Debug, Clone, Copy)]
pub(crate) enum GateActor<'a> {
    /// The signed-in caller, with the request's headers: the tenant is
    /// the `X-Tenant` header when the caller sent one, else the
    /// caller's first tenant. `SEC-20`'s rule, unchanged.
    Caller {
        principal: &'a Principal,
        headers: &'a HeaderMap,
    },
    /// A named tenant, resolved as if it had been sent in `X-Tenant`:
    /// the site acts as that tenant, and a principal that does not
    /// belong to it is refused with the same 404 an `X-Tenant` header
    /// naming a foreign tenant gets. `None` means the site has no
    /// tenant at all, and reaches nothing.
    ///
    /// `SEC-50` R1: a principal holding the unrestricted (`"*:*"`) grant
    /// passes this actor TOO, in both cases — with a tenant and without
    /// one, whether or not the principal belongs to that tenant. That is
    /// `SEC-20`'s own signed rule ("a principal holding `*:*` passes"),
    /// and it is safe here for the same reason it is safe in Query
    /// Studio: an unrestricted principal already reads everything there,
    /// so a statement running as that principal reaches no more than the
    /// principal itself reaches.
    AsTenant {
        principal: &'a Principal,
        tenant_id: Option<Uuid>,
    },
    /// The deployment's own catalog tenant: a job with no user (an
    /// alert, the scheduled Gold export, an embed token).
    CatalogTenant,
}

/// `SEC-50`: the ONE gate every statement passes before it is rewritten.
///
/// `SEC-20` put the tenant gate inside the query path; this is that
/// gate, lifted out of it so that every site which runs a statement
/// through [`crate::policy_engine::rewrite_sql_for_roles`] asks it
/// first — the query path, the authored pipelines, the Gold export, and
/// (from slice 2) the dashboards, the embeds, the public links and the
/// alerts. The gate is the same code for all of them; only the actor
/// differs, and the actor says whose reach the statement is measured
/// against.
///
/// The targets come from ONE parse, with the dialect of the engine the
/// statement names (the same choice the policy rewrite makes): the
/// tables the statement names, or the fact that it lists objects
/// without naming one. A statement that does not parse has no targets
/// at all: the policy rewrite refuses it with a 422 of its own, so
/// there is nothing here to gate.
///
/// # Errors
///
/// 404 with the fixed sentence of [`table_not_available`] for a table
/// the actor may not read; 422 with the fixed sentence of
/// [`LISTING_NOT_AVAILABLE`] for a statement that lists objects and the
/// actor may not list them; 404 for a tenant the actor may not act as;
/// 422 with the fixed sentence of [`JOB_NEEDS_CATALOG_TENANT`] for a
/// job with no user in a deployment that holds several tenants and no
/// `CATALOG_TENANT_ID`; 503 when the deployment's own answers cannot be
/// read.
pub(crate) async fn gate_statement<D>(
    state: &AppState,
    sql: &str,
    dialect: &D,
    actor: GateActor<'_>,
) -> Result<(), ApiError>
where
    D: sqlparser::dialect::Dialect + Sync,
{
    let Some(targets) = crate::sql_rewrite::gate_targets(sql, dialect) else {
        // A statement that does not parse has no targets: the policy
        // rewrite refuses it with a 422 of its own, so there is nothing
        // here to gate.
        return Ok(());
    };
    refuse_unreachable_tables(state, actor, &targets).await
}

/// `SEC-50`: the reach of an actor — whose tables the statement may
/// read.
///
/// Each actor answers the same question with the answers the service
/// already has (the shared catalog's own gate, the acting tenant, the
/// deployment's tenant count, and the setting that names the catalog's
/// tenant), never with a rule of its own.
///
/// # Errors
///
/// 404 for a tenant the actor may not act as; 422 with the fixed
/// sentence of [`JOB_NEEDS_CATALOG_TENANT`] for a job with no user in a
/// deployment that holds several tenants and no `CATALOG_TENANT_ID`;
/// 503 when the deployment's own answers cannot be read.
async fn reach_for_actor(state: &AppState, actor: GateActor<'_>) -> Result<TenantReach, ApiError> {
    match actor {
        GateActor::Caller { principal, headers } => resolve_reach(state, principal, headers).await,
        GateActor::AsTenant {
            principal,
            tenant_id,
        } => {
            // `SEC-50` R1 (review round 1, BLOCKER 1): the unrestricted
            // grant is checked FIRST, before the tenant is looked at at
            // all. `SEC-20`'s own rule is that a principal holding
            // `*:*` passes, and it is the same rule here: an
            // unrestricted principal reads every table in Query Studio
            // already, so a statement that runs as that principal
            // reaches no more than the principal itself reaches.
            //
            // Checked first, this also answers the case of a site with
            // NO tenant of its own (a pipeline whose `tenant_id` is
            // NULL): an unrestricted author reads everything, so the
            // pipeline is not refused for the tenant it never had.
            // Without this, the G7 harness's bootstrap-admin pipelines
            // (created by a `*:*` admin that belongs to no tenant) would
            // be refused for a tenant they never had.
            if crate::routes::catalog::is_unrestricted(principal) {
                return Ok(TenantReach::All);
            }
            match tenant_id {
                // D5: the site has no tenant at all (a pipeline whose own
                // `tenant_id` is NULL). It reaches nothing — the
                // migration's own fail-closed rule.
                None => Ok(TenantReach::NoTenant),
                // D5: the tenant is checked and resolved exactly as an
                // `X-Tenant` header is, so the reach is the one the
                // principal would have with that header — one rule, not
                // two.
                Some(tenant_id) => {
                    let headers = crate::tenant_scope::headers_for_tenant(tenant_id)?;
                    resolve_reach(state, principal, &headers).await
                }
            }
        }
        GateActor::CatalogTenant => {
            let tenant_count = deployment_tenant_count(state).await?;
            catalog_tenant_reach(tenant_count, state.config.catalog_tenant_id)
        }
    }
}

/// `SEC-50` D3: the reach of a job with no user, as a pure function of
/// the two answers it needs.
///
/// The job reads the SHARED catalog (`serving.*`, Silver, Gold), so the
/// only tenant whose reach can cover it is the tenant that owns that
/// catalog — which is why the job acts as the catalog's tenant and not
/// as any person.
///
/// The reach is `All` when the deployment holds a single tenant (there is
/// no other tenant's table to reach), the catalog's own tenant when
/// `CATALOG_TENANT_ID` names it, and a refusal otherwise: a deployment
/// with several tenants and no catalog tenant has no tenant whose reach
/// covers the shared catalog, and failing closed is the only honest
/// answer (D6).
///
/// `SEC-50` R5 (amended, PR #281 review round 2 NIT): a deployment that
/// cannot COUNT its tenants refuses too — and with `SEC-20`'s OWN sentence
/// for that state,
/// [`crate::routes::catalog::COULD_NOT_VERIFY_TENANT_COUNT`] ("could not
/// verify tenant count"), not D6's. D6 names a setting to set, and that is
/// the wrong instruction here: the setting may ALREADY be set (naming a
/// catalog tenant in a deployment that cannot count its tenants is a claim
/// nobody can check), and what is missing is the deployment's own answer,
/// not the setting.
///
/// `None` means there is no database to count in, so the deployment cannot
/// say that it holds a single tenant — and it cannot say that it holds
/// several either. "Cannot verify it is safe" is the same answer as
/// "verified unsafe", which is `SEC-20`'s own rule: the shared catalog's
/// own gate refuses every principal without the unrestricted grant when
/// there is no pool (`routes::catalog::catalog_tenant_refusal`).
fn catalog_tenant_reach(
    tenant_count: Option<i64>,
    catalog_tenant_id: Option<Uuid>,
) -> Result<TenantReach, ApiError> {
    let Some(tenant_count) = tenant_count else {
        return Err(ApiError::Unavailable(
            crate::routes::catalog::COULD_NOT_VERIFY_TENANT_COUNT.to_owned(),
        ));
    };
    if tenant_count <= 1 {
        return Ok(TenantReach::All);
    }
    match catalog_tenant_id {
        Some(tenant_id) => Ok(TenantReach::Tenant {
            tenant_id,
            owns_catalog: true,
        }),
        None => Err(ApiError::Unprocessable(JOB_NEEDS_CATALOG_TENANT.to_owned())),
    }
}

/// `SEC-50` D6: the one sentence a job with no user is refused with when
/// the deployment holds several tenants and no tenant owns the shared
/// catalog.
///
/// It names the setting and what to do with it, and says nothing about
/// the deployment's data (`AGENTS.md` principle 4).
pub(crate) const JOB_NEEDS_CATALOG_TENANT: &str = "This job reads the shared catalog, and no tenant owns it here. Set CATALOG_TENANT_ID to the tenant that owns this deployment's shared catalog.";

/// `SEC-20`: how far the caller's reach extends over the tables a
/// statement reads.
///
/// One decision per request, resolved by [`resolve_reach`] before any
/// table is looked at, so "may this caller read that table" is asked
/// once and answered the same way for every table and every route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TenantReach {
    /// Nothing is out of reach: a principal holding every permission
    /// (`"*:*"`), or a deployment with one tenant (or none) — there is no
    /// other tenant's table to reach, so the gate changes nothing.
    All,
    /// The caller's own tenant, and whether that tenant owns the shared
    /// catalog (the tenant named in `CATALOG_TENANT_ID`).
    Tenant { tenant_id: Uuid, owns_catalog: bool },
    /// The caller belongs to no tenant in a deployment that has more than
    /// one: it owns no table and may read no shared one.
    NoTenant,
}

/// `SEC-20`: the one sentence a table the caller may not read is refused
/// with.
///
/// A 404, never a 403: a 403 would confirm the table exists to a caller
/// who must not learn that, and Query Studio reads a 403 as "this user
/// may not run anything at all" and refuses the whole batch. The
/// sentence is the same whether the table exists and belongs to somebody
/// else or does not exist at all, so the refusal is not an oracle
/// either.
fn table_not_available(table: &str) -> ApiError {
    ApiError::NotFound(format!(
        "Table not found or not available to your organisation: {table}"
    ))
}

/// `SEC-20`: the one sentence a statement that lists objects without
/// naming one table is refused with.
///
/// A 422, never a 403: the statement is refused for what it DOES (it
/// lists), not because the caller lacks a permission, and the sentence
/// says nothing about what the deployment holds — the same sentence for
/// every caller, so it cannot be an oracle.
///
/// A listing is refused rather than passed because no table is named, so
/// the gate has nothing to compare the caller's tenant against: letting
/// it through would show one tenant the shared catalog's table names.
const LISTING_NOT_AVAILABLE: &str = "This statement is not available to your organisation.";

/// `SEC-20`: the caller's reach over tables, resolved once per request.
///
/// The answers it composes are the ones the rest of the service already
/// uses, not copies of them (`AGENTS.md` rule 4):
/// [`crate::routes::catalog::is_unrestricted`] (the `"*:*"` grant), the
/// shared catalog's own gate
/// [`crate::routes::catalog::catalog_tenant_refusal`],
/// [`crate::tenant_scope::resolve`] (the `X-Tenant` header, or the
/// caller's first tenant), and [`deployment_tenant_count`] (whether the
/// deployment holds a second tenant at all).
///
/// # Errors
///
/// [`ApiError::NotFound`] when `X-Tenant` names a tenant the caller does
/// not belong to — the same fail-closed 404 the tenant scope answers
/// everywhere else, never a 403 that would confirm the tenant exists.
async fn resolve_reach(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<TenantReach, ApiError> {
    if crate::routes::catalog::is_unrestricted(principal) {
        return Ok(TenantReach::All);
    }
    let refused = crate::routes::catalog::catalog_tenant_refusal(state, principal, headers)
        .await?
        .is_some();
    let tenant = crate::tenant_scope::resolve(principal, headers)?;
    // The count is read only when the gate admitted the caller: a
    // refusal decides the reach on its own, and the query path is hot.
    //
    // It is read AFTER the gate's own read, so the later answer is the
    // one that decides: a deployment that gained a tenant between the
    // two reads is scoped, not unscoped.
    let tenant_count = if refused {
        None
    } else {
        deployment_tenant_count(state).await?
    };
    Ok(reach_from(
        refused,
        tenant,
        tenant_count,
        state.config.catalog_tenant_id,
    ))
}

/// `SEC-20`: the reach, as a pure function of the answers the request has.
///
/// The answers are the shared catalog's own gate (refused or not), the
/// caller's tenant ([`crate::tenant_scope::resolve`]), how many tenants
/// the deployment holds, and which tenant owns the shared catalog
/// (`CATALOG_TENANT_ID`).
///
/// An answer that cannot be confirmed narrows the reach. A gate that
/// admitted the caller is not proof that the caller owns the shared
/// catalog: the gate admits a caller for two reasons (the caller owns the
/// catalog, or the deployment holds a single tenant), and the count it
/// read may be older than this request's. So the ownership is checked
/// against the setting that names the catalog's tenant, and the count is
/// read again. A deployment that grew a tenant in between is scoped,
/// never unscoped — the narrowing is the safe direction, because the
/// shared tables and the other tenants' uploads are the things to keep.
fn reach_from(
    refused: bool,
    tenant: Option<Uuid>,
    tenant_count: Option<i64>,
    catalog_tenant_id: Option<Uuid>,
) -> TenantReach {
    let one_tenant = tenant_count.is_some_and(|count| count <= 1);
    match (refused, tenant) {
        // The shared catalog is open to this caller.
        (false, Some(tenant_id)) => {
            if one_tenant {
                // A deployment with a single tenant holds no other
                // tenant's table to reach: the gate changes nothing.
                TenantReach::All
            } else if catalog_tenant_id == Some(tenant_id) {
                // The caller is the tenant that owns the shared catalog.
                TenantReach::Tenant {
                    tenant_id,
                    owns_catalog: true,
                }
            } else {
                // The gate admitted the caller, but the caller is not the
                // tenant that owns the shared catalog: the gate must have
                // read a smaller count than this request did. Fail
                // closed — the shared tables are not the caller's.
                TenantReach::Tenant {
                    tenant_id,
                    owns_catalog: false,
                }
            }
        }
        // The gate admitted a caller that belongs to no tenant, so it
        // read a deployment with a single tenant.
        (false, None) => {
            if one_tenant {
                TenantReach::All
            } else {
                // The deployment grew a tenant since the gate read its
                // count: a caller that belongs to no tenant owns no table
                // and reads no shared one.
                TenantReach::NoTenant
            }
        }
        // The shared catalog is closed to this caller: only the uploads
        // its own tenant claimed are its own.
        (true, Some(tenant_id)) => TenantReach::Tenant {
            tenant_id,
            owns_catalog: false,
        },
        // A caller with no tenant in a deployment that has more than
        // one: it owns no upload and may read no shared table.
        (true, None) => TenantReach::NoTenant,
    }
}

/// `SEC-20`: the saved queries the caller may read.
///
/// The same reach the same request resolves for the tables a statement
/// may read (see [`resolve_reach`]), asked once and answered once: the
/// caller's tenant, whether that tenant owns the shared catalog, and
/// whether the deployment holds a single tenant at all. A saved query
/// belongs to the tenant that wrote it, so the reach over tables and the
/// reach over saved queries end in the same place — and there is only
/// one rule that says which tenant a request acts as
/// ([`crate::tenant_scope::resolve`]).
///
/// # Errors
///
/// The same as [`resolve_reach`]: 404 for an `X-Tenant` header the
/// caller does not belong to, 503 when the count cannot be read.
pub(crate) async fn saved_query_scope(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
) -> Result<queries::SavedQueryScope, ApiError> {
    Ok(match resolve_reach(state, principal, headers).await? {
        // Every permission, or a deployment with one tenant: every row.
        TenantReach::All => queries::SavedQueryScope::All,
        // One tenant's rows; the rows that belong to no tenant are the
        // shared catalog's own, and the catalog's tenant reads them.
        TenantReach::Tenant {
            tenant_id,
            owns_catalog,
        } => queries::SavedQueryScope::Tenant {
            tenant_id,
            include_tenantless: owns_catalog,
        },
        // A caller in no tenant owns no saved query and reads none.
        TenantReach::NoTenant => queries::SavedQueryScope::None,
    })
}

/// `SEC-20`: how many tenants the deployment holds.
///
/// `None` when there is no database to count: the shared catalog's own
/// gate refuses every caller it cannot verify in that state, and this gate
/// fails closed with it.
///
/// The count is read beside the gate's own answer, never instead of it.
/// The gate answers "may this caller read the shared catalog"; this
/// answers "does this deployment hold a second tenant at all". The two
/// are different questions, and the first does not tell the second: the
/// gate admits a caller for the catalog's own tenant AND for a
/// deployment with a single tenant, and those two have to end differently
/// — a deployment with one tenant changes nothing.
///
/// # Errors
///
/// 503 when the count cannot be read: the store's own fixed message and a
/// reference id, through [`upstream_error`] (the raw error text never
/// reaches the response, `AGENTS.md` principle 4).
async fn deployment_tenant_count(state: &AppState) -> Result<Option<i64>, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(None);
    };
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM tenant")
        .fetch_one(pool)
        .await
        .map_err(|err| {
            upstream_error::report_sqlx(&upstream_error::STORE, &err)
                .into_api_error(upstream_error::FailedAs::ServiceUnavailable)
        })?;
    Ok(Some(count))
}

/// `SEC-20`: the rule, as a pure function of the reach and the claim on
/// the table.
///
/// - [`TenantReach::All`]: everything is reachable.
/// - [`TableClaim::Ours`]: the caller's own tenant holds the table (an
///   upload it made) — reachable, whoever owns the shared catalog.
/// - [`TableClaim::Theirs`]: another tenant holds it — never reachable.
/// - [`TableClaim::Unclaimed`]: nobody holds it — a table no upload
///   claimed, or a shared table — and it is reachable only by the
///   catalog's own tenant.
fn table_is_reachable(reach: TenantReach, claim: TableClaim) -> bool {
    match reach {
        TenantReach::All => true,
        TenantReach::NoTenant => false,
        TenantReach::Tenant { owns_catalog, .. } => match claim {
            TableClaim::Ours => true,
            TableClaim::Theirs => false,
            TableClaim::Unclaimed => owns_catalog,
        },
    }
}

/// `SEC-20`: the raw upload table a canonical table name names, when it
/// names one.
///
/// An upload lands in the Bronze namespace under its raw name
/// (`upload_table_claim.bronze_table`, the key
/// [`lakehouse_store::uploads::table_claim`] reads), so `bronze.orders`
/// is the upload table `orders` and no other namespace holds an upload.
/// A bare one-part name is not it: `orders` alone names whatever the
/// caller's default database holds.
fn raw_table_name(table: &str) -> Option<&str> {
    table
        .strip_prefix(lakehouse_iceberg::bronze::BRONZE_NAMESPACE)?
        .strip_prefix('.')
}

/// `SEC-20`: who holds `table`, as the caller's own tenant sees it.
///
/// A table no upload claimed has no holder, so it is nobody's and the
/// shared rule decides it. The read is the store's own
/// [`lakehouse_store::uploads::table_claim`] — the same answer the
/// ingest path's `TABLE_NOT_FREE` conflict is built on, never a second
/// reading of `upload_table_claim`.
///
/// A deployment with no Postgres pool has no claims to read, and the
/// shared catalog's own gate refuses every principal without the
/// unrestricted grant in that state anyway, so the answer is "nobody
/// holds it" and the shared rule decides.
///
/// # Errors
///
/// 503 when the claim cannot be read: the store's own fixed message and
/// a reference id, through [`upstream_error`] (the raw error text never
/// reaches the response, `AGENTS.md` principle 4).
async fn claim_on(state: &AppState, tenant_id: Uuid, table: &str) -> Result<TableClaim, ApiError> {
    let Some(raw) = raw_table_name(table) else {
        return Ok(TableClaim::Unclaimed);
    };
    let Some(pool) = state.pg.as_deref() else {
        return Ok(TableClaim::Unclaimed);
    };
    lakehouse_store::uploads::table_claim(pool, tenant_id, raw)
        .await
        .map_err(|err| {
            upstream_error::store(&upstream_error::STORE, &err)
                .into_api_error(upstream_error::FailedAs::ServiceUnavailable)
        })
}

/// `SEC-20`/`SEC-50`: refuse a statement the actor's tenant may not run.
///
/// The targets are what the statement means to the gate: the tables it
/// names — canonicalised the way the policy rewrite canonicalises them
/// ([`crate::sql_rewrite::canonicalize`]), so a `CTE`'s own name is not a
/// table — or the fact that it lists objects without naming one table.
///
/// A named table is checked one by one, and the first table out of reach
/// is the one the refusal names. A listing names no table, so there is
/// nothing to check table by table: it passes for the catalog's own
/// tenant (and for a deployment with one tenant) and is refused
/// otherwise.
///
/// The actor's reach is resolved only when there is something to check:
/// a statement that names no table reads nothing, so it is let through
/// without asking the deployment anything (`SEC-20`'s behaviour, kept).
///
/// # Errors
///
/// 404 with the fixed sentence of [`table_not_available`] for a table the
/// actor may not read; 422 with the fixed sentence of
/// [`LISTING_NOT_AVAILABLE`] for a statement that lists objects and the
/// actor may not list them; the errors of [`reach_for_actor`] — 404 for a
/// tenant the actor may not act as, 422 for a job with no user in a
/// deployment with several tenants and no `CATALOG_TENANT_ID`; 503 when
/// a claim cannot be read.
async fn refuse_unreachable_tables(
    state: &AppState,
    actor: GateActor<'_>,
    targets: &crate::sql_rewrite::GateTargets,
) -> Result<(), ApiError> {
    match targets {
        crate::sql_rewrite::GateTargets::Tables(tables) => {
            if tables.is_empty() {
                return Ok(());
            }
            let reach = reach_for_actor(state, actor).await?;
            let tenant_id = match reach {
                TenantReach::All => return Ok(()),
                TenantReach::NoTenant => return Err(table_not_available(&tables[0])),
                TenantReach::Tenant { tenant_id, .. } => tenant_id,
            };
            let mut claims = Vec::with_capacity(tables.len());
            for table in tables {
                claims.push(claim_on(state, tenant_id, table).await?);
            }
            match first_unreachable(reach, &claims) {
                Some(at) => Err(table_not_available(&tables[at])),
                None => Ok(()),
            }
        }
        crate::sql_rewrite::GateTargets::ListsObjects => {
            // The reach is resolved FIRST: a header naming a tenant the
            // caller does not belong to is a 404 everywhere else, and it
            // must not be answered with the listing's own 422.
            let reach = reach_for_actor(state, actor).await?;
            match reach {
                // Every permission, or a deployment with one tenant: there
                // is no other tenant's listing to keep. The catalog's own
                // tenant may list the shared catalog.
                TenantReach::All
                | TenantReach::Tenant {
                    owns_catalog: true, ..
                } => Ok(()),
                // A member of another tenant, or a caller in no tenant.
                TenantReach::Tenant {
                    owns_catalog: false,
                    ..
                }
                | TenantReach::NoTenant => {
                    Err(ApiError::Unprocessable(LISTING_NOT_AVAILABLE.to_owned()))
                }
            }
        }
    }
}

/// `SEC-20`: the first table the caller may not read, in the order the
/// tables were listed.
///
/// The rule itself is [`table_is_reachable`]; this only says which table
/// the refusal names, so the decision stays a pure function of the reach
/// and the claims.
fn first_unreachable(reach: TenantReach, claims: &[TableClaim]) -> Option<usize> {
    claims
        .iter()
        .position(|claim| !table_is_reachable(reach, *claim))
}

/// The tables `sql` names, canonicalised the way the policy rewrite
/// canonicalises them and sorted, so the refusal always names the same
/// table for the same statement.
///
/// A statement that does not parse has no table list: the policy rewrite
/// refuses it with a 422 of its own, so there is nothing here to gate.
fn referenced_table_names(sql: &str, trino: bool) -> Vec<String> {
    let dialect: &dyn sqlparser::dialect::Dialect = if trino {
        &sqlparser::dialect::GenericDialect {}
    } else {
        &sqlparser::dialect::ClickHouseDialect {}
    };
    let mut tables = crate::sql_rewrite::referenced_tables(sql, dialect).unwrap_or_default();
    tables.sort();
    tables
}

/// Fixed text for `DATA-16` F3: past versions are queried on `ClickHouse`
/// only (the `Trino` side is `DATA-21`).
const PAST_VERSION_TRINO_MESSAGE: &str =
    "Querying a past version of a table is available on the ClickHouse engine only.";

/// Whether `sql`'s tokens contain the keyword run `VERSION AS OF` or
/// `TIMESTAMP AS OF`. Reads tokens (not the raw text) so the same words in
/// a string literal, a quoted identifier or a comment are not mistaken for
/// the clause. A tokenizer failure answers `false`: the caller then keeps
/// today's message.
fn uses_past_version_clause(sql: &str) -> bool {
    use sqlparser::tokenizer::{Token, Tokenizer};

    let dialect = sqlparser::dialect::GenericDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, sql).tokenize() else {
        return false;
    };
    // Whitespace and comments carry no meaning between the three words.
    let words: Vec<String> = tokens
        .into_iter()
        .filter_map(|t| match t {
            Token::Word(w) if w.quote_style.is_none() => Some(w.value.to_ascii_uppercase()),
            Token::Whitespace(_) => None,
            // Any other token breaks a run of keywords.
            _ => Some(String::new()),
        })
        .collect();
    words
        .windows(3)
        .any(|w| (w[0] == "VERSION" || w[0] == "TIMESTAMP") && w[1] == "AS" && w[2] == "OF")
}

/// The one answer to a statement the settings rule refuses. It names no word
/// of the statement and no upstream text (`AGENTS.md` principle 4).
const SETTINGS_REFUSAL_MESSAGE: &str =
    "Query Studio accepts only iceberg_snapshot_id in SETTINGS; the server sets the others.";

/// The one setting a statement may name: the version picker's
/// (`src/lib/snapshot-picker.ts`, `DATA-16`).
const ALLOWED_SETTING: &str = "iceberg_snapshot_id";

/// Finds a `SETTINGS` clause other than [`ALLOWED_SETTING`] anywhere in a
/// parsed statement. The `Visitor` below reaches every `Query` (a
/// subquery, a CTE, a derived table, an expression subquery, the query
/// inside `EXPLAIN`), every table function's own argument list and every
/// `INSERT`, which are the three places `sqlparser` keeps settings. The
/// derived `Visit` covers every node kind, so a kind added by a later
/// `sqlparser` is reached too.
struct SettingsFinder;

impl SettingsFinder {
    fn refuses(settings: Option<&[sqlparser::ast::Setting]>) -> std::ops::ControlFlow<()> {
        match settings {
            Some(list) if list.iter().any(|s| s.key.value != ALLOWED_SETTING) => {
                std::ops::ControlFlow::Break(())
            }
            _ => std::ops::ControlFlow::Continue(()),
        }
    }
}

impl sqlparser::ast::Visitor for SettingsFinder {
    type Break = ();

    fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> std::ops::ControlFlow<()> {
        Self::refuses(query.settings.as_deref())
    }

    fn pre_visit_table_factor(
        &mut self,
        table_factor: &sqlparser::ast::TableFactor,
    ) -> std::ops::ControlFlow<()> {
        match table_factor {
            sqlparser::ast::TableFactor::Table {
                args: Some(args), ..
            } => Self::refuses(args.settings.as_deref()),
            _ => std::ops::ControlFlow::Continue(()),
        }
    }

    fn pre_visit_statement(
        &mut self,
        statement: &sqlparser::ast::Statement,
    ) -> std::ops::ControlFlow<()> {
        match statement {
            sqlparser::ast::Statement::Insert(insert) => Self::refuses(insert.settings.as_deref()),
            _ => std::ops::ControlFlow::Continue(()),
        }
    }
}

/// SEC-21: a statement may name one setting, `iceberg_snapshot_id`. Any
/// other `SETTINGS` entry would override the request-level settings the run
/// sends (`ChClient::query_limited`): `SETTINGS max_result_rows = 0`
/// measured 500,000 of 500,000 rows past the cap on `ClickHouse` 26.8, and
/// `readonly = 1` is not an option because it also refuses the picker's
/// setting.
///
/// `sql` is the statement that goes to the engine (the rewritten one), so
/// what is checked is what `ClickHouse` reads. It is parsed with
/// `ClickHouseDialect`; a statement that does not parse, or that is not
/// exactly one statement, is refused with the same message, because a text
/// this check cannot read is a text it cannot vouch for. Names are compared
/// case-sensitively, so `ICEBERG_SNAPSHOT_ID` is refused.
///
/// # Errors
/// 422 [`ApiError::Unprocessable`] with [`SETTINGS_REFUSAL_MESSAGE`] when
/// the statement names another setting, does not parse, or is more than one
/// statement. The parser's own message is dropped.
fn refuse_unlisted_settings(sql: &str) -> Result<(), ApiError> {
    let refusal = || ApiError::Unprocessable(SETTINGS_REFUSAL_MESSAGE.to_owned());
    let statements =
        sqlparser::parser::Parser::parse_sql(&sqlparser::dialect::ClickHouseDialect {}, sql)
            .map_err(|_| refusal())?;
    if statements.len() != 1 {
        return Err(refusal());
    }
    match sqlparser::ast::Visit::visit(&statements, &mut SettingsFinder) {
        std::ops::ControlFlow::Break(()) => Err(refusal()),
        std::ops::ControlFlow::Continue(()) => Ok(()),
    }
}

/// Runs the already-rewritten `sql` against whichever engine `engine`
/// names, returning `(columns, rows, duration_ms, scanned_bytes)`. Split
/// out of [`run`] purely to keep that function under
/// `clippy::too_many_lines` after WS7 item C2 added the rewrite step —
/// no behavior change from the inline block it replaces.
async fn execute_query_against_engine(
    state: &AppState,
    engine: &str,
    sql: &str,
    user_id: &str,
    started: Instant,
    (run, limits): (
        &crate::query_limits::QueryRun,
        &crate::query_limits::TileLimits,
    ),
) -> Result<(Vec<String>, Vec<Value>, u64, u64), ApiError> {
    if engine == "trino" {
        let trino_user = format!("console-{user_id}");
        let result = state
            .trino
            .run_statement(sql, &trino_user)
            .await
            .map_err(map_trino_error)?;
        let (columns, rows) = trino_result_to_rows(&result);
        Ok((columns, rows, elapsed_ms(started), 0))
    } else {
        // SEC-11, query-author exception (product owner, 2026-10-10): the
        // signed-in author of this statement, holding `query:read`, gets the
        // engine's diagnosis of it (version and addresses trimmed); the full
        // text is logged under the reference. An unreachable engine or a
        // non-query failure stays the fixed message. See
        // `upstream_error::ch_error_for_author`.
        //
        // SEC-21: the engine stops producing rows past the cap, so a huge
        // result never reaches this process. One row more than the route
        // keeps is asked for, so `truncated` stays honest.
        //
        // BI-5: the cap is the administrator's Query Studio row limit, the
        // engine also stops the query at the time limit, and the query is
        // named so Stop can end it (`QueryRun::options`). A query the limit
        // or a Stop ended gets a fixed sentence (plan D6), not the engine's
        // text.
        let options = run.options(limits, limits.studio_rows);
        let result = state
            .clickhouse
            .query_with_options(sql, &options, None)
            .await
            .map_err(|e| {
                if e.is_time_limit() {
                    ApiError::Unprocessable(crate::query_limits::time_sentence("query", limits))
                } else if e.is_killed_on_engine() {
                    ApiError::Unprocessable(crate::query_limits::STOPPED_SENTENCE.to_owned())
                } else {
                    upstream_error::ch_error_for_author(&upstream_error::DATABASE, &e)
                }
            })?;

        let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
        let rows: Vec<Value> = result
            .data
            .iter()
            .map(|row| {
                let mut out = Map::new();
                for c in &columns {
                    let v = row.get(c);
                    out.insert(c.clone(), Value::String(stringify_cell(v)));
                }
                Value::Object(out)
            })
            .collect();

        let scanned_bytes = result.statistics.as_ref().map_or(0, |s| s.bytes_read);
        let duration_ms = result
            .statistics
            .as_ref()
            .map_or_else(|| elapsed_ms(started), |s| seconds_to_ms(s.elapsed));
        Ok((columns, rows, duration_ms, scanned_bytes))
    }
}

/// Answers whether `sql` would run for `principal` on `engine`, without
/// running it. Plan 15's proposal check: a statement a person could be
/// shown as a fix must pass the same guard and policy `run` applies, and
/// must plan on `ClickHouse`.
///
/// The order matches `run`: [`is_read_only`] first, then
/// [`rewrite_sql_for_principal`], so the statement that is planned is the
/// one `run` would execute. On `ClickHouse` the rewritten statement is sent
/// as `EXPLAIN PLAN` with its trailing `;` stripped, the same shape
/// `estimate_body` uses for `EXPLAIN ESTIMATE`. `EXPLAIN PLAN` resolves
/// column names, which `EXPLAIN AST` does not. Only one request is sent,
/// and it is an `EXPLAIN`.
///
/// # Errors
///
/// Returns the message for the statement's author, as text. It is never an
/// [`ApiError`] and reaches no HTTP response body from here; the caller
/// hands it to a tool, which cleans it.
/// - A statement that is not read-only: a fixed sentence naming that only
///   read statements can be proposed.
/// - A statement the policy refuses: the policy's own non-leaking message,
///   from [`rewrite_sql_for_principal`].
/// - A statement that names a setting other than `iceberg_snapshot_id`: the
///   settings rule's fixed sentence (`SEC-21`).
/// - A statement `ClickHouse` cannot plan: its own SQL error text, which is
///   the author's to see, the same text `run_sql` already treats as such.
/// - `engine` `"trino"`: a fixed "not supported yet" sentence. Plan 16
///   replaces this arm.
pub(crate) async fn check_statement_would_run(
    state: &AppState,
    principal: &Principal,
    sql: &str,
    engine: &str,
) -> Result<(), String> {
    if !is_read_only(sql) {
        return Err(
            "Only read statements (SELECT/SHOW/DESCRIBE/EXPLAIN) can be proposed.".to_owned(),
        );
    }
    let rewritten = rewrite_sql_for_principal(state, sql, engine, principal)
        .await
        .map_err(|err| err.to_string())?;
    if is_trino_engine(engine) {
        return Err("Checking a Trino statement is not supported yet.".to_owned());
    }
    // SEC-21: the check sends the statement to the engine, so the proposal
    // must meet the settings rule a run applies, or the user would be shown a
    // fix that `run` then refuses.
    refuse_unlisted_settings(&rewritten).map_err(|err| err.to_string())?;
    let query = format!("EXPLAIN PLAN {}", strip_trailing_semicolon(&rewritten));
    state
        .clickhouse
        .query(&query, None)
        .await
        .map(|_| ())
        // SEC-11, merged from main (`propose_sql`, #127): the engine's text
        // goes through the same query-author exception as `run` and the cost
        // estimate (trimmed statement diagnosis for the author; an outage
        // stays the fixed message with a reference id).
        .map_err(|err| upstream_error::ch_message_for_author(&upstream_error::DATABASE, &err))
}

// The most rows one run hands back to the browser is the administrator's
// Query Studio row limit (`TileLimits::studio_rows`, default 64,000;
// `BI-5`, `QS-2`).
//
// The console renders every row it receives, so an unbounded `SELECT *`
// used to be enough to freeze the tab. The response says when the cap
// applied (`truncated`), so nobody reads a partial answer as the whole one.
//
// `ClickHouse` is asked to stop at the cap (`ChClient::query_with_options`),
// so a huge result does not fill this process's memory before `truncate`
// runs (SEC-21). `result_overflow_mode = 'break'` stops at a block
// boundary, so up to one block (`max_block_size`, about 65,000 rows by
// default) beyond the cap can still arrive; the route keeps `studio_rows` of
// them. `Trino`'s client bounds its own result.

// The most bytes one result may carry to the browser: 10 MB (10 * 1024 * 1024 bytes)
// of cell text.
//
// The row limit bounds how many rows arrive; this bounds how much they
// weigh. A result of few, wide rows passes the row limit and still fills
// the browser's memory, so the bytes are capped too (`QS-2`). The cap
// is applied after the row cut and cuts at the last whole row that fits, so
// a result is never half a row.
const RESULT_BYTE_CAP: usize = 10 * 1024 * 1024;

/// `POST /api/query/run` — execute a read-only `SQL` statement against
/// `ClickHouse` (`engine` absent or `"clickhouse"`) or `Trino`
/// (`engine: "trino"`, WS2 §4) and return it in `QueryResult` shape.
///
/// Every run that reaches an answer is recorded, best-effort (see
/// `queries::record_history`'s doc comment): a completed run in the audit
/// trail and in the caller's query history, as before; a refused run as
/// history status `blocked` and a failed engine call as `failed`, each
/// with the exact message the response carries and the tables the
/// statement names (`QS-4`). A run whose body does not parse, whose `sql`
/// is empty, or that has no principal is not recorded — nobody, or
/// nothing, to record. A run the caller stops is recorded as `failed`
/// too (`BI-5`): Stop ends the statement on the server, so the server is
/// told, and the row carries the stop's own fixed sentence. The audit
/// trail's own refusal path stays what WS7 built it for: enforcement
/// decisions, not this route's history write.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] on an unparseable body or a missing/empty
///   `sql`.
/// - 401 [`ApiError::Unauthorized`] with no authenticated principal, for
///   ANY engine. `query_history.user_name` now records the principal's own
///   uuid (previously the fixed placeholder `"anonymous"` — see
///   `QueryHistoryItem::user`'s doc comment), and `engine: "trino"`'s
///   `X-Trino-User` is derived from that same uuid, never
///   `principal.display_name` (a display name is attacker-controlled, and
///   the compose `trino` service's file-based access control grants full
///   `iceberg` catalog write access to exactly the user name
///   `trino-maintenance` — see that service's `rules.json` block comment).
///   Neither has a safe/honest value to fall back to with no principal, so
///   the request is refused before anything runs. The route is
///   policy-gated (`Policy::RequiresPermission("query:read")`), so this is
///   defense in depth, the same shape `routes::gold::export` uses.
/// - 404 [`ApiError::NotFound`] when the statement reads a table the
///   caller's tenant may not read: the fixed sentence of
///   [`table_not_available`], the same whether the table belongs to
///   another tenant or does not exist at all (`SEC-20`). Never a 403:
///   Query Studio's Run all reads a 403 as "this user may not run
///   anything" and would refuse the other statements too.
/// - 422 [`ApiError::Unprocessable`] when `sql` fails [`is_read_only`] (any
///   engine) or [`is_trino_safe`] (`engine: "trino"` only), or when the
///   engine itself rejects the query, or `sql_rewrite::enforce` refuses it
///   (WS7 item C2 — a table function, a sensitive `system.*` read, a
///   `dictGet`/`joinGet`-family call, a view over a governed table, or an
///   authored-but-unenforceable policy condition, WS7 plan Hard
///   Requirement 1), or, on `ClickHouse`, when the statement names a setting
///   other than `iceberg_snapshot_id` (`SEC-21`).
/// - 503 [`ApiError::Unavailable`] when `Trino` is unreachable or times out.
#[allow(
    clippy::too_many_lines,
    reason = "one linear guard order (read-only, principal, Trino keywords, \
              policy rewrite, settings, engine), each guard now ending in the \
              same best-effort history record that `record_blocked` and \
              `record_uncompleted_run` carry; splitting the guards into \
              helpers would scatter an order whose sequence is the route's \
              correctness, for a threshold, not a real seam"
)]
pub async fn run(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let statement = match parsed.sql {
        Some(s) if !s.is_empty() => s,
        _ => return Err(ApiError::BadRequest("sql is required".to_owned()).into()),
    };
    // The engine is only a request field, so reading it before the guards
    // changes nothing about which refusal a request meets. It is needed
    // here for `tables_read`: the statement's tables are parsed with the
    // dialect of the engine it names (the same parser the policy step
    // uses), and the history row records them whether the run completes,
    // fails or is refused (`QS-4`, decisions 3 and 4).
    let engine = parsed.engine.as_deref().unwrap_or("clickhouse").to_owned();
    let tables_read = referenced_table_names(&statement, is_trino_engine(&engine));

    if !is_read_only(&statement) {
        let refusal = ApiError::Unprocessable(
            "Only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query Studio."
                .to_owned(),
        );
        // No principal, no row: nobody to record the refusal against
        // (`QS-4`, decision 3).
        if let Some(Extension(p)) = &principal {
            record_blocked(&state, p, &statement, &engine, &tables_read, &refusal).await;
        }
        return Err(refusal.into());
    }

    // See this function's doc comment: there is no honest value left to
    // record (or, for Trino, to send as `X-Trino-User`) for an
    // unauthenticated caller, so every engine requires a principal now —
    // `"anonymous"` is never written to `query_history` again.
    let Some(Extension(p)) = principal else {
        return Err(ApiError::unauthorized().into());
    };
    let user_id = p.id.uuid().to_string();
    // BI-5: this caller's limits and the run (`X-Run-Id`) the statement
    // belongs to, so Stop can find it.
    let run = crate::query_limits::QueryRun::current_or_detached();
    let limits = run.limits().await?;
    let row_limit = limits.studio_rows as usize;

    if engine == "trino" && !is_trino_safe(&statement) {
        let refusal = ApiError::Unprocessable(
            "This statement is not permitted against Trino (write, session, or procedure \
             keyword detected)."
                .to_owned(),
        );
        record_blocked(&state, &p, &statement, &engine, &tables_read, &refusal).await;
        return Err(refusal.into());
    }

    // WS7 item C2: every ad hoc SQL surface is rewritten through the same
    // `sql_rewrite::enforce` (Phase B) before it reaches EITHER engine —
    // still after `is_read_only`, per this route's existing ordering, and
    // before `started`/`Instant::now()` below so the elapsed time recorded
    // for `policyDecisionP95Ms` covers exactly the rewrite step, nothing
    // else.
    //
    // `statement` stays what the user wrote: query history records it (and
    // `download` re-rewrites it for the caller's roles at that time), while
    // `rewritten` is what the engine runs and the audit event records
    // (`SEC-21`, `SEC-33`).
    let rewritten =
        match rewrite_sql_for_principal_with_headers(&state, &statement, &engine, &p, &headers)
            .await
        {
            Ok(rewritten) => rewritten,
            Err(err) => {
                // A 4xx from the rewrite is the gate refusing (the policy, the
                // tenant's tables, the caller's own tenant header); a 5xx is
                // the service failing while asking. The history row says
                // which.
                let status: &'static str = if err.status() < 500 {
                    "blocked"
                } else {
                    "failed"
                };
                record_uncompleted_run(
                    &state,
                    &p,
                    &statement,
                    &engine,
                    status,
                    &err.to_string(),
                    &tables_read,
                    0,
                )
                .await;
                return Err(err.into());
            }
        };
    // SEC-21: a statement's own `SETTINGS` would override the cap the run
    // sends with the request. Trino has no such settings in its client.
    if !is_trino_engine(&engine)
        && let Err(refusal) = refuse_unlisted_settings(&rewritten)
    {
        record_blocked(&state, &p, &statement, &engine, &tables_read, &refusal).await;
        return Err(refusal.into());
    }

    let started = Instant::now();
    let started_epoch_ms = epoch_ms();

    // QS-4: every run that reaches the engine is recorded, whether it
    // answered or not. BI-5's limits travel with the call, so a run the time
    // limit or a Stop ended ends here with the limit's own fixed sentence —
    // and that sentence, not the engine's text, is what the history row
    // carries, exactly as the response does.
    let (columns, mut rows, duration_ms, scanned_bytes) = match execute_query_against_engine(
        &state,
        &engine,
        &rewritten,
        &user_id,
        started,
        (&run, &limits),
    )
    .await
    {
        Ok(result) => result,
        Err(err) => {
            record_uncompleted_run(
                &state,
                &p,
                &statement,
                &engine,
                "failed",
                &err.to_string(),
                &tables_read,
                started.elapsed().as_millis(),
            )
            .await;
            return Err(err.into());
        }
    };

    // The console renders every row it receives, so an unbounded `SELECT *`
    // used to be enough to freeze the tab — see the note above this function.
    // The engines stop at the cap (`ClickHouse` inside the engine,
    // `Trino` in its client); this keeps exactly `row_limit` of what
    // arrived, and the response says when it did.
    let total_rows = rows.len();
    let cut_by_rows = total_rows > row_limit;
    rows.truncate(row_limit);

    // `QS-2`: the byte cap, after the row cut. The row limit bounds how
    // many rows arrive; a result of few, wide rows passes it and still
    // weighs more than the browser should take, so the bytes are capped as
    // well. The cap cuts at the last whole row that fits and never empties a
    // result.
    let (rows, cut_by_bytes) = cut_rows_by_bytes(rows, RESULT_BYTE_CAP);
    // The byte cap is applied last, so it is the limit that decided how many
    // rows the browser got; when it cut, it is the one the response names.
    // The row limit is named when only it cut.
    let cut_by = if cut_by_bytes {
        Some("bytes")
    } else if cut_by_rows {
        Some("rows")
    } else {
        None
    };

    let cost_units = std::cmp::max(1, bytes_to_cost_units(scanned_bytes));
    let id = history_id(started_epoch_ms);

    // Record a real audit_event for this run — best-effort, the same
    // non-fatal posture as the history write just below: the query already
    // ran and the caller already has their result, so a failure here is
    // logged and swallowed, never turned into an error response. Written
    // via `lakehouse_store::audit::insert` directly (not
    // `routes::ai::audit::record`, which hardcodes `principal_kind:
    // "copilot"` — wrong here, a query run is human- or service-triggered,
    // never copilot-triggered). `resource_kind: "query_history"` paired
    // with this run's own `id` is the pairing WS1 T16's `LEFT JOIN`
    // resolves on read (WS5 Phase D preamble).
    if let Some(pool) = state.pg.as_deref() {
        let event = query_run_audit_event(&p, &id, &rewritten);
        if let Err(err) = lakehouse_store::audit::insert(pool, event).await {
            tracing::warn!(%err, "failed to record query.run audit event (query itself succeeded)");
        }
    }

    // Record this execution in query history — best-effort. A history-write
    // failure (no Postgres pool configured, Postgres down, ...) must never
    // turn an otherwise-successful query into an error response for the
    // caller, who already has their result: log a warning and keep going.
    // See `lakehouse_store::queries::record_history`'s doc comment.
    //
    // `user` is the principal's own uuid (guaranteed present above) and
    // `engine` is the engine that actually ran — both genuine measurements
    // now, not the pre-WS2-§4 `"anonymous"`/`"hot-store"` placeholders.
    if let Some(pool) = state.pg.as_deref() {
        #[allow(
            clippy::cast_possible_wrap,
            clippy::cast_precision_loss,
            reason = "scanned_bytes/duration_ms/cost_units are ClickHouse/Trino-reported sizes \
                      for one query, well within i64/f64's exact-integer range"
        )]
        let input = RecordHistoryInput {
            id: &id,
            sql: &statement,
            user: &user_id,
            owner_id: Some(p.id.uuid()),
            status: "completed",
            duration_ms: duration_ms as i64,
            scanned_bytes: scanned_bytes as i64,
            cost_units: cost_units as f64,
            workload_class: "hot-analytics",
            engine: &engine,
            cache_assisted: false,
            tables_read: &tables_read,
            message: None,
        };
        if let Err(err) = queries::record_history(pool, &input).await {
            tracing::warn!(%err, "failed to record query history (query itself succeeded)");
        }
    }

    Ok(ApiJson(run_result_json(
        &id,
        &json!(columns),
        &json!(rows),
        (total_rows, row_limit),
        cut_by,
        duration_ms,
        scanned_bytes,
        cost_units,
        &engine,
    )))
}

/// `QS-4` (the history page, decision 3): the one place records a run
/// that did not complete.
///
/// The read-only guard, the Trino keyword check, the policy rewrite, the
/// tenant gate and the settings guard record `blocked`; the engine's own
/// error records `failed`. `message` is the exact text the response
/// carries — an `ApiError`'s response body is `{"error": err.to_string()}`
/// (`error.rs`), so the row and the caller's screen can never disagree,
/// and the engine's own text reaches the row only the way it reached the
/// response: already fixed and classified. Best-effort, the same posture
/// as the completed write: a failed write is logged and never turns into
/// an error response.
///
/// A refusal carries no measured numbers — nothing executed — so its
/// duration is 0 and nothing was scanned or spent. An engine failure
/// carries the time the failed attempt took, measured by the caller,
/// where `started` is already in scope; nothing was scanned and nothing
/// was spent that anyone reported.
#[allow(
    clippy::cast_possible_truncation,
    reason = "an attempt's elapsed milliseconds are wall-clock milliseconds; \
              a duration that outlives i64's range outlives the service"
)]
async fn record_uncompleted_run(
    state: &AppState,
    principal: &Principal,
    statement: &str,
    engine: &str,
    status: &'static str,
    message: &str,
    tables_read: &[String],
    duration_ms: u128,
) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let user_id = principal.id.uuid().to_string();
    let id = history_id(epoch_ms());
    let input = RecordHistoryInput {
        id: &id,
        sql: statement,
        user: &user_id,
        owner_id: Some(principal.id.uuid()),
        status,
        duration_ms: duration_ms as i64,
        scanned_bytes: 0,
        cost_units: 0.0,
        workload_class: "hot-analytics",
        engine,
        cache_assisted: false,
        tables_read,
        message: Some(message),
    };
    if let Err(err) = queries::record_history(pool, &input).await {
        tracing::warn!(%err, "failed to record the refused or failed run");
    }
}

/// The guard's shape of [`record_uncompleted_run`]: a refusal before the
/// engine is asked is `blocked`, took no time, and its message is the
/// refusal the response carries. One wrapper, so the guards' call sites
/// stay one line and the five of them cannot drift apart.
async fn record_blocked(
    state: &AppState,
    principal: &Principal,
    statement: &str,
    engine: &str,
    tables_read: &[String],
    refusal: &ApiError,
) {
    record_uncompleted_run(
        state,
        principal,
        statement,
        engine,
        "blocked",
        &refusal.to_string(),
        tables_read,
        0,
    )
    .await;
}

/// Built once per query run, unit-tested without a `ClickHouse`/Postgres
/// connection — the pure half of the audit write, matching this file's
/// existing pattern for pure helpers (`is_read_only`, `bytes_to_cost_units`,
/// ...). `principal_kind` comes from [`Principal::kind_for_audit`]
/// (`Principal.id`'s variant), never from `principal.provider` — see that
/// method's doc comment for the `0024_audit_event.sql` CHECK it satisfies.
/// `args.sql` reuses `routes::ai::audit`'s own truncation so an
/// unbounded-length `SQL` string can't bloat `audit_event.args`.
fn query_run_audit_event(principal: &Principal, query_id: &str, sql: &str) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: "query.run".to_owned(),
        resource_kind: Some("query_history".to_owned()),
        resource_id: Some(query_id.to_owned()),
        args: Some(json!({ "sql": crate::routes::ai::audit::truncate_string(sql) })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    }
}

/// Coerce one cell to the string-everything shape both `run_result_json`
/// row shapes use, matching the `TypeScript`'s implicit stringification.
fn stringify_cell(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(other) => other.to_string(),
    }
}

/// Convert a [`TrinoResult`] into the same `(columns, rows)` shape the
/// `ClickHouse` branch of [`run`] produces, so both engines answer with an
/// identical `QueryResult` row format.
fn trino_result_to_rows(result: &TrinoResult) -> (Vec<String>, Vec<Value>) {
    let columns: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let rows: Vec<Value> = result
        .rows
        .iter()
        .map(|row| {
            let mut out = Map::new();
            for (i, c) in columns.iter().enumerate() {
                out.insert(c.clone(), Value::String(stringify_cell(row.get(i))));
            }
            Value::Object(out)
        })
        .collect();
    (columns, rows)
}

/// Map a [`TrinoError`] to the [`ApiError`] `run` returns.
///
/// SEC-11: [`TrinoError::Query`] is the one variant whose engine text
/// reaches the caller, trimmed and with a reference id (the query-author
/// exception, product owner decision 2026-10-10: the person who wrote the
/// statement is entitled to the engine's diagnosis of it). Timeouts,
/// transport and HTTP failures stay fixed messages.
fn map_trino_error(err: TrinoError) -> ApiError {
    match err {
        // SEC-11, query-author exception (product owner, 2026-10-10): `Query`
        // is Trino answering that the statement is wrong, and this route's
        // caller wrote the statement; trimmed, with the reference.
        TrinoError::Query(msg) => upstream_error::query_error_for_author(
            &upstream_error::Context::new(
                "trino query",
                "The Trino query failed.",
                "Trino is unavailable.",
            ),
            &msg,
        ),
        TrinoError::Timeout => ApiError::Unavailable("trino query timed out".to_owned()),
        TrinoError::Transport(_) | TrinoError::HttpStatus(_) => {
            ApiError::Unavailable("trino unavailable".to_owned())
        }
        TrinoError::TooManyRows { cap } => {
            ApiError::Unprocessable(format!("trino result exceeded the {cap}-row cap"))
        }
    }
}

/// The byte cost of one cell: the length of its text — a JSON string's own
/// bytes without the quotes, any other value's compact JSON text — plus one
/// byte for the separator (`QS-2`).
fn cell_byte_cost(cell: &Value) -> usize {
    let text = match cell {
        Value::String(text) => text.len(),
        other => other.to_string().len(),
    };
    text + 1
}

/// The byte cost of one row: the sum of its cells' costs.
///
/// The run path builds object rows (one field per column, see
/// [`execute_query_against_engine`]); an array row (one element per column)
/// is counted the same way, so the rule does not depend on the row shape.
fn row_byte_cost(row: &Value) -> usize {
    match row {
        Value::Object(cells) => cells.values().map(cell_byte_cost).sum(),
        Value::Array(cells) => cells.iter().map(cell_byte_cost).sum(),
        other => cell_byte_cost(other),
    }
}

/// Cut `rows` to the last whole row that fits in `cap` bytes.
///
/// Returns the rows to keep and whether the cap cut anything. A first row
/// that does not fit is kept anyway: the byte cap never empties a result
/// (`QS-2`) — one enormous row is still an answer.
fn cut_rows_by_bytes(rows: Vec<Value>, cap: usize) -> (Vec<Value>, bool) {
    let mut kept: Vec<Value> = Vec::with_capacity(rows.len());
    let mut total = 0usize;
    let mut cut = false;
    for row in rows {
        let cost = row_byte_cost(&row);
        if !kept.is_empty() && total.saturating_add(cost) > cap {
            cut = true;
            break;
        }
        total = total.saturating_add(cost);
        kept.push(row);
    }
    (kept, cut)
}

/// Build the `/api/query/run` response body from already-computed values.
///
/// Extracted from [`run`] so the transparency-panel contract — which fields
/// are real measurements versus `null` — is unit-testable without a live
/// `ClickHouse` connection.
///
/// `engine` is the top-level field naming which engine actually ran this
/// statement (`"clickhouse"`/`"trino"`, WS2 §4) — a DIFFERENT thing from
/// `metrics.engine`, which stays the fixed string `"hot-store"` describing
/// `ClickHouse`'s storage tier regardless of which top-level engine ran.
/// Neither renames nor removes the other.
///
/// `cut_by` names the limit that cut the result (`"rows"` for the Query
/// Studio row limit, `"bytes"` for the 10 MB cap) and is absent when
/// nothing was cut: the key is then not in the JSON at all, not `null`.
/// `truncated` is derived from it, so the two can never disagree.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat result row, no natural grouping"
)]
fn run_result_json(
    id: &str,
    columns: &Value,
    rows: &Value,
    (total_rows, row_limit): (usize, usize),
    cut_by: Option<&str>,
    duration_ms: u64,
    scanned_bytes: u64,
    cost_units: u64,
    engine: &str,
) -> Value {
    let mut body = json!({
        "id": id,
        "engine": engine,
        "columns": columns,
        "rows": rows,
        "rowCount": total_rows,
        "truncated": cut_by.is_some(),
        "rowLimit": row_limit,
        "metrics": {
            "durationMs": duration_ms,
            "scannedBytes": scanned_bytes,
            "costUnits": cost_units,
            "engine": "hot-store",
            "workloadClass": "hot-analytics",
            // WS1 task 1.6: `ClickHouse`'s response carries no cache-hit flag
            // and no pushdown list, and there is no policy engine to produce
            // obligations (`WS7` builds one). Null says "not measured"; the
            // old constants (`false`, `[]`, `[]`) claimed a measurement that
            // was never taken.
            "cacheHit": Value::Null,
            "pushdowns": Value::Null,
            "policyObligations": Value::Null,
        },
        // WS1 task 1.6: the old `plan` was a single hardcoded stage claiming
        // "scan + aggregate" and status "completed" for every query,
        // regardless of what `ClickHouse` actually did. Null until a real
        // plan is computed.
        "plan": Value::Null,
    });
    // The body is the object the macro above built, so this always finds
    // it; the `let` keeps the `unwrap` out of the code (AGENTS.md, Rust).
    if let Some(cut_by) = cut_by
        && let Some(object) = body.as_object_mut()
    {
        object.insert("cutBy".to_owned(), json!(cut_by));
    }
    body
}

/// Current time as Unix milliseconds, matching JavaScript's `Date.now()`
/// used to build the `q-<ms>` id.
#[allow(
    clippy::cast_possible_truncation,
    reason = "milliseconds since epoch fits comfortably in i64 until year 292 million"
)]
fn epoch_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// The id a recorded run carries: `q-<epoch_ms>-<uuid simple>`.
///
/// The epoch milliseconds keep the id readable and roughly ordered; the
/// uuid's simple form — 32 lowercase hex characters — makes it unique.
/// The store inserts the history row with `ON CONFLICT (id) DO NOTHING`, so
/// two runs whose ids collided would leave one of the two rows out of the
/// history, and the epoch milliseconds alone collide whenever two runs
/// land in the same millisecond.
///
/// One place mints this id, so the two callers cannot drift apart: the
/// answered run and the refused or failed one.
fn history_id(epoch_ms: i64) -> String {
    format!("q-{epoch_ms}-{}", Uuid::new_v4().simple())
}

/// Wall-clock duration since `started`, in milliseconds — the fallback
/// `Date.now() - started` timing path when `ClickHouse` reports no
/// `statistics.elapsed`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "a single request's wall-clock duration in ms cannot exceed u64 range in practice"
)]
fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

/// `Math.round(elapsedSeconds * 1000)`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "ClickHouse-reported elapsed seconds for one query is always a small, \
              non-negative value in practice"
)]
fn seconds_to_ms(elapsed_seconds: f64) -> u64 {
    (elapsed_seconds * 1000.0).round() as u64
}

/// `Math.round(scannedBytes / 1_000_000)` — "~1 unit / MB read".
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    reason = "byte counts here are well within f64's exact-integer range \
              (2^53), and the ratio is always small and non-negative"
)]
fn bytes_to_cost_units(scanned_bytes: u64) -> u64 {
    (scanned_bytes as f64 / 1_000_000.0).round() as u64
}

/// Four decimal places, which is the resolution the console prints.
fn round4(value: f64) -> f64 {
    (value * 10_000.0).round() / 10_000.0
}

/// The min (÷2M) and max (÷1M) cost buckets in `query/estimate`, to the
/// same four decimals as [`bytes_to_cost_units`].
#[allow(
    clippy::cast_precision_loss,
    reason = "estimated byte counts here are well within f64's exact-integer range (2^53)"
)]
fn bytes_to_cost_bucket(estimated_bytes: i64, divisor: f64) -> f64 {
    round4(estimated_bytes as f64 / divisor)
}

/// `POST /api/query/estimate` — a rough cost/plan estimate via `EXPLAIN
/// ESTIMATE`, never erroring back to the caller.
///
/// # Errors
///
/// Returns 400 [`ApiError::BadRequest`] on an unparseable body or a
/// missing/blank `sql`, and 422 [`ApiError::Unprocessable`] when `sql` fails
/// [`is_read_only`], the rewrite refuses it, or it names a setting other than
/// `iceberg_snapshot_id` (`SEC-21`), as [`run`] does. An `EXPLAIN
/// ESTIMATE` failure (e.g. an unknown column) still answers 200 — the
/// console shows the estimate panel either way — but `estimatedBytes`/costs
/// are `null` and the `ClickHouse` message is carried in `error`, rather
/// than a zeroed estimate that looked identical to "this query reads
/// nothing".
pub async fn estimate(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed = parse_body(&body)?;
    let sql = match parsed.sql {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Err(ApiError::BadRequest("sql is required".to_owned()).into()),
    };
    // SEC-19: the estimate refuses what the run refuses, with the run's
    // status and message, and does so before the rewrite below, whose policy
    // prefetch already queries the engine. `EXPLAIN ESTIMATE` is the only
    // statement this route may send, so a non-read never reaches it.
    if !is_read_only(&sql) {
        return Err(ApiError::Unprocessable(
            "Only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) are allowed in Query Studio."
                .to_owned(),
        )
        .into());
    }

    // A failed EXPLAIN is reported, not swallowed. It used to come back as
    // a zeroed estimate, so "this query cannot even be planned" and "this
    // query reads nothing" looked identical — and ClickHouse's message is
    // usually the most useful thing on the page (an unknown column, a
    // missing table) before anyone presses Run.
    //
    // Review fix: the estimate explains the statement the run would send, i.e.
    // the one rewritten for the caller's current roles (same function as the
    // run), and a statement the rewrite refuses is refused here with the run's
    // status and fixed message and is never sent to the engine. The planner
    // reports the tables the statement reads, which the rewrite does not
    // change, so the sources shown are what the run would show.
    let rewritten =
        rewrite_sql_for_principal_with_headers(&state, &sql, "clickhouse", &principal, &headers)
            .await?;
    // SEC-21: the same settings rule as the run, on the statement the engine
    // is sent.
    refuse_unlisted_settings(&rewritten)?;
    let estimated = estimate_body(&state.clickhouse, &rewritten).await;
    let (estimated_bytes, sources, estimate_error) = match estimated {
        Ok((bytes, sources)) => (Some(bytes), sources, None),
        Err(message) => (None, Vec::new(), Some(message)),
    };
    let freshness_lag_seconds = freshness_lag(&state.clickhouse, &sources).await;

    // No floor: an estimate of nothing costs nothing. `max(1, …)` used to
    // make an empty editor read "1.00 cu–1.00 cu".
    let cost_min = estimated_bytes.map(|b| bytes_to_cost_bucket(b, 2_000_000.0));
    let cost_max = estimated_bytes.map(|b| bytes_to_cost_bucket(b, 1_000_000.0));
    let sources_out: Vec<String> = if sources.is_empty() {
        vec!["clickhouse@lakehouse".to_owned()]
    } else {
        sources
    };

    let mut result = estimate_result_json(
        estimated_bytes,
        cost_min,
        cost_max,
        &json!(sources_out),
        freshness_lag_seconds,
        estimate_error.as_deref(),
    );
    // DATA-12 part 3: the marks of the tables this ClickHouse query reads,
    // beside `sources` (which stays a list of names). One batch read; only a
    // marked source gets an entry, and `sourceMarks` is absent when none is.
    // This route estimates ClickHouse queries only; a Trino query is not
    // estimated here and has no sources to mark.
    // PR #106 review R1 BLOCKER 2, SHOULD-FIX DATA-12 2:
    // Only decorate with sourceMarks if catalog_tenant_refusal allows it:
    if let Ok(None) =
        crate::routes::catalog::catalog_tenant_refusal(&state, &principal, &headers).await
    {
        let names = std::collections::HashMap::new();
        let marks = crate::routes::catalog::certification_marks(&state, &sources_out, &names).await;
        if !marks.is_empty()
            && let Some(o) = result.as_object_mut()
        {
            o.insert("sourceMarks".to_owned(), json!(marks));
        }
    }
    Ok(ApiJson(result))
}

/// Build the `/api/query/estimate` response body from already-computed
/// values. See [`run_result_json`] for why this is extracted.
#[allow(
    clippy::too_many_arguments,
    reason = "one flat result row, no natural grouping"
)]
fn estimate_result_json(
    estimated_bytes: Option<i64>,
    cost_min: Option<f64>,
    cost_max: Option<f64>,
    sources_out: &Value,
    freshness_lag_seconds: Option<i64>,
    error: Option<&str>,
) -> Value {
    json!({
        "estimatedBytes": estimated_bytes,
        "estimatedCostMin": cost_min,
        "estimatedCostMax": cost_max,
        "error": error,
        // True by construction: Query Studio runs everything through
        // ClickHouse, the hot analytical store.
        "workloadClass": "hot-analytics",
        "engine": "hot-store",
        // Nothing in this deployment configures ClickHouse's query cache,
        // and no policy engine is consulted before a run (`WS7` builds
        // one) — `cacheEligible`/`policyObligations` stay `null`, not a
        // fabricated "Eligible"/`[]`. `freshnessLagSeconds` IS a real
        // measurement (`freshness_lag`, ClickHouse `system.parts`), unlike
        // the `0` it used to hardcode.
        "cacheEligible": Value::Null,
        "freshnessLagSeconds": freshness_lag_seconds,
        "policyObligations": Value::Null,
        "sources": sources_out,
        // WS1 task 1.6: the old `plan` marked stages that had *not run* as
        // status "completed" with `estimatedBytes: 0` — a fabricated
        // measurement of work that was never done. Null until a real plan
        // is computed.
        "plan": Value::Null,
    })
}

/// Seconds since the newest write to any table the query reads, or `None`
/// when that cannot be established — no source tables identified, or
/// `ClickHouse` did not answer.
///
/// The lag reported is the *worst* one across the sources: a join is only
/// as fresh as its stalest side. This used to be hardcoded to 0, so the
/// panel said "Fresh · 0 s" for every query, including one that had not
/// been written yet.
async fn freshness_lag(ch: &ChClient, sources: &[String]) -> Option<i64> {
    let qualified: Vec<&String> = sources.iter().filter(|s| s.contains('.')).collect();
    if qualified.is_empty() {
        return None;
    }
    let predicate = qualified
        .iter()
        .filter_map(|s| s.split_once('.'))
        .map(|(db, table)| {
            format!(
                "(database = '{}' AND table = '{}')",
                escape_literal(db),
                escape_literal(table)
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");
    if predicate.is_empty() {
        return None;
    }
    let sql = format!(
        "SELECT toString(dateDiff('second', max(modification_time), now())) lag \
         FROM system.parts WHERE active AND ({predicate})"
    );
    let rows = ch.rows(&sql, None).await.ok()?;
    rows.first()?
        .get("lag")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<i64>().ok())
}

/// Single-quote escaping for a `ClickHouse` string literal. The names come
/// from `EXPLAIN ESTIMATE`'s own output rather than from the caller, but
/// they are still interpolated into SQL, so they are escaped.
fn escape_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Runs `EXPLAIN ESTIMATE <sql>` (with any trailing `;` stripped) and
/// tallies `estimatedBytes`/`sources` from the result rows, or returns a
/// fixed message and a reference id when `ClickHouse` could not plan the
/// statement.
async fn estimate_body(ch: &ChClient, sql: &str) -> Result<(i64, Vec<String>), String> {
    let trimmed = strip_trailing_semicolon(sql);
    let query = format!("EXPLAIN ESTIMATE {trimmed}");
    // SEC-11, query-author exception (product owner, 2026-10-10): the editor
    // shows the author the planner's diagnosis of their own statement,
    // trimmed; an outage stays the fixed message.
    let result = ch
        .query(&query, None)
        .await
        .map_err(|err| upstream_error::ch_message_for_author(&upstream_error::DATABASE, &err))?;
    let mut estimated_bytes: i64 = 0;
    let mut sources = Vec::new();
    for row in &result.data {
        let db = row.get("database").and_then(Value::as_str).unwrap_or("");
        let tbl = row.get("table").and_then(Value::as_str).unwrap_or("");
        if !tbl.is_empty() {
            sources.push(if db.is_empty() {
                tbl.to_owned()
            } else {
                format!("{db}.{tbl}")
            });
        }
        let rows_n = row.get("rows").and_then(numeric_value).unwrap_or(0.0);
        estimated_bytes += rows_to_bytes(rows_n);
    }
    Ok((estimated_bytes, sources))
}

/// `rows * 64` — the rough byte-per-row estimate `EXPLAIN ESTIMATE` rows
/// are converted to.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "row counts here are well within f64's exact-integer range and \
              always non-negative"
)]
fn rows_to_bytes(rows: f64) -> i64 {
    (rows * 64.0) as i64
}

/// Coerce a `serde_json::Value` cell to `f64`, matching JavaScript's
/// `Number(row["rows"] ?? 0)`.
fn numeric_value(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// `sql.replace(/;\s*$/, "")` — strip one trailing `;` (and any trailing
/// whitespace after it), not every trailing `;`.
fn strip_trailing_semicolon(sql: &str) -> &str {
    let trimmed_end = sql.trim_end();
    trimmed_end.strip_suffix(';').unwrap_or(trimmed_end)
}

// ── Postgres-backed writes (Task 2.4) ───────────────────────────────────
//
// `listSaved`/`listHistory` back the methods `src/services/clients/queries.ts`
// used to delegate to `mockQueryService`. No TypeScript server-side
// precedent exists for either — like `routes::identity`/
// `routes::governance`'s Postgres-backed additions, status codes are chosen
// to be correct rather than faithful.

/// Borrow the Postgres pool, or fail with a 503 explaining why there isn't
/// one. Mirrors `routes::identity::pool`.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "query store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// `GET /api/query/saved` — the caller's tenant's saved queries.
///
/// `SEC-20`: a saved query is a statement somebody wrote against their
/// own data, so it belongs to the tenant that wrote it. The list holds
/// the caller's tenant's rows, and the rows that belong to no tenant
/// when that tenant owns the shared catalog (see [`saved_query_scope`]).
///
/// # Errors
///
/// 401 when nobody is signed in — a saved query belongs to a tenant, so
/// there is no sensible answer for "everybody's"; 404 for an `X-Tenant`
/// header the caller does not belong to; 503 if no pool is configured;
/// 500 on a database failure.
pub async fn list_saved(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Vec<SavedQuery>>> {
    let Extension(principal) =
        principal.ok_or_else(|| ApiError::Unauthorized("sign in required".to_owned()))?;
    let scope = saved_query_scope(&state, &principal, &headers).await?;
    Ok(ApiJson(queries::list_saved(pool(&state)?, scope).await?))
}

/// The `GET /api/query/history` query string (`QS-4`, decision 5). Every
/// field stays a raw string on purpose: this struct parses whatever the
/// caller sends, so a bad *value* becomes a 400 with this route's own
/// fixed message in [`parse_history_query`] — never `serde_urlencoded`'s
/// rendering of a serde error. A query string the extractor itself cannot
/// read (a parameter sent twice) never reaches this struct: it is refused
/// with the fixed sentence of [`HISTORY_QUERY_UNREADABLE`].
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryQueryRaw {
    status: Option<String>,
    engine: Option<String>,
    min_duration_ms: Option<String>,
    table: Option<String>,
    from: Option<String>,
    to: Option<String>,
    /// The page cursor: `<at>,<id>` — the `at` and `id` of the last row
    /// the caller has already seen.
    before: Option<String>,
}

/// The statuses `query_history`'s own check allows — the set [`run`]
/// records, and nothing else.
const HISTORY_STATUSES: [&str; 4] = ["completed", "failed", "cancelled", "blocked"];

/// The engines [`run`] records.
const HISTORY_ENGINES: [&str; 2] = ["clickhouse", "trino"];

/// What a query string the route cannot read is told.
///
/// The twin of this sentence lives in `routes/uploads.rs`
/// (`QUERY_UNREADABLE`). The two cannot be one constant yet: the place
/// both could share is `routes/mod.rs`, and that file is held by another
/// open pull request.
const HISTORY_QUERY_UNREADABLE: &str = "The query string could not be read.";

/// Parse the history query string into a [`HistoryFilter`], refusing a
/// bad value with a fixed 400 before the store sees it.
///
/// Unknown keys are ignored, not refused: the contract is the seven
/// documented names, and a caller sending more than that is forward
/// compatible, not wrong. An empty value (`?status=`) is no value, so a
/// client that builds links from empty inputs filters nothing instead of
/// getting a 400 it cannot read.
fn parse_history_query(raw: &HistoryQueryRaw) -> Result<HistoryFilter<'_>, ApiError> {
    let mut filter = HistoryFilter::default();
    if let Some(status) = raw.status.as_deref().filter(|s| !s.is_empty()) {
        if !HISTORY_STATUSES.contains(&status) {
            return Err(ApiError::BadRequest(
                "status must be one of completed, failed, cancelled, blocked".to_owned(),
            ));
        }
        filter.status = Some(status);
    }
    if let Some(engine) = raw.engine.as_deref().filter(|s| !s.is_empty()) {
        if !HISTORY_ENGINES.contains(&engine) {
            return Err(ApiError::BadRequest(
                "engine must be one of clickhouse, trino".to_owned(),
            ));
        }
        filter.engine = Some(engine);
    }
    if let Some(min) = raw.min_duration_ms.as_deref().filter(|s| !s.is_empty()) {
        filter.min_duration_ms = Some(min.parse::<i64>().ok().filter(|ms| *ms >= 0).ok_or_else(
            || {
                ApiError::BadRequest(
                    "minDurationMs must be a non-negative whole number of milliseconds".to_owned(),
                )
            },
        )?);
    }
    if let Some(table) = raw.table.as_deref().filter(|s| !s.is_empty()) {
        filter.table = Some(table);
    }
    filter.from = history_time("from", raw.from.as_deref())?;
    filter.to = history_time("to", raw.to.as_deref())?;
    if let Some(before) = raw.before.as_deref().filter(|s| !s.is_empty()) {
        let bad_before = || {
            ApiError::BadRequest(
                "before must be <at>,<id>: the at and the id of the last row you have seen"
                    .to_owned(),
            )
        };
        let (at, id) = before.split_once(',').ok_or_else(bad_before)?;
        if id.is_empty() {
            return Err(bad_before());
        }
        filter.before_at = Some(OffsetDateTime::parse(at, &Rfc3339).map_err(|_| bad_before())?);
        filter.before_id = Some(id);
    }
    Ok(filter)
}

/// Parse one of the window bounds: RFC 3339, the format the rows' own
/// `at` carries, so a cursor cut from a listed row parses back.
fn history_time(field: &str, raw: Option<&str>) -> Result<Option<OffsetDateTime>, ApiError> {
    match raw {
        // An empty value is no value, like every other filter field.
        None | Some("") => Ok(None),
        Some(raw) => OffsetDateTime::parse(raw, &Rfc3339)
            .map(Some)
            .map_err(|_| ApiError::BadRequest(format!("{field} must be an RFC 3339 time"))),
    }
}

/// `GET /api/query/history` — the caller's own recorded query executions,
/// filtered, inside the last 14 days, one page at a time (`QS-4`).
///
/// # Errors
///
/// 401 when nobody is signed in — history belongs to a person, so there is
/// no sensible answer for "everyone's". 400 for a bad filter value, with a
/// fixed message naming the field, and for a query string the route cannot
/// read (a parameter sent twice), with the fixed sentence of
/// [`HISTORY_QUERY_UNREADABLE`]. 503 if no pool is configured; 500 on a
/// database failure.
pub async fn list_history(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    query: Result<Query<HistoryQueryRaw>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<ApiJson<Vec<QueryHistoryItem>>> {
    let owner = caller_id(principal.as_ref())?;
    // A query string the extractor cannot read — a parameter sent twice, for
    // instance — is refused with this route's own sentence. The rejection's
    // own text is serde's, and it stays in the log: a response never
    // carries a library's rendering of the caller's request.
    let Query(raw) = query.map_err(|err| {
        tracing::warn!(%err, "a query history query string could not be read");
        ApiError::BadRequest(HISTORY_QUERY_UNREADABLE.to_owned())
    })?;
    let filter = parse_history_query(&raw)?;
    Ok(ApiJson(
        queries::list_history(pool(&state)?, owner, &filter).await?,
    ))
}

/// The signed-in caller's id, or a 401.
fn caller_id(principal: Option<&Extension<Principal>>) -> Result<Uuid, ApiError> {
    principal
        .map(|Extension(p)| p.id.uuid())
        .ok_or_else(|| ApiError::Unauthorized("sign in required".to_owned()))
}

/// The `POST /api/query/saved` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSavedQueryBody {
    title: String,
    sql: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// `POST /api/query/saved` — save the SQL in the editor under a name.
///
/// `SEC-20`: a saved query belongs to the tenant that wrote it. The
/// tenant is the one the request acts as — the `X-Tenant` header when the
/// caller sends one, the caller's first tenant otherwise — and the row is
/// that tenant's alone; the rows that belong to no tenant are the shared
/// catalog's own. The author is recorded all the same.
///
/// # Errors
///
/// 400 on a malformed body or a blank title/SQL; 401 when nobody is signed
/// in; 404 for an `X-Tenant` header the caller does not belong to; 422
/// when the SQL is not read-only — a saved query is run through the same
/// guard as anything else, so refusing it here is better than storing
/// something that can never run; 503/500 as above.
pub async fn create_saved(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<(StatusCode, ApiJson<SavedQuery>)> {
    let body: CreateSavedQueryBody = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("invalid JSON: {err}")))?;
    let title = body.title.trim().to_owned();
    let sql = body.sql.trim().to_owned();
    if title.is_empty() || sql.is_empty() {
        return Err(ApiError::BadRequest("title and sql are required".to_owned()).into());
    }
    if !is_read_only(&sql) {
        return Err(ApiError::Unprocessable(
            "only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) can be saved".to_owned(),
        )
        .into());
    }
    let owner_id = caller_id(principal.as_ref())?;
    // The tenant the row belongs to is the tenant the request acts as,
    // answered by the one resolver every caller without a header gets
    // (`tenant_scope::resolve`). A caller in no tenant writes a row that
    // belongs to no tenant: nobody can say whose query it is, so it is
    // not anybody's to read but the shared catalog's owner's.
    let tenant_id = crate::tenant_scope::resolve(
        principal
            .as_ref()
            .map(|Extension(p)| p)
            .ok_or_else(|| ApiError::Unauthorized("sign in required".to_owned()))?,
        &headers,
    )?;
    let owner = principal.map_or_else(|| "anonymous".to_owned(), |Extension(p)| p.display_name);
    let saved = queries::create_saved_query(
        pool(&state)?,
        &title,
        &sql,
        &owner,
        &body.tags,
        Some(owner_id),
        tenant_id,
    )
    .await?;
    Ok((StatusCode::CREATED, ApiJson(saved)))
}

// ── GET /api/query/run/{id}/download (QS-2) ─────────────────────────────

/// What the console may ask a download for.
#[derive(Debug, Deserialize)]
pub(crate) struct DownloadQuery {
    /// The file kind: `csv`, `tsv`, `xlsx` or `parquet`.
    format: String,
    /// The run the download belongs to. Stop ends the queries of one run by
    /// their name (`<caller uuid>:<runId>:%`), so the download names its
    /// own query the same way and Stop reaches it.
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

/// The file kinds a Query Studio download can be (`QS-2`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadFormat {
    /// UTF-8 text with a byte-order mark, its fields separated by a comma.
    Csv,
    /// UTF-8 text with a byte-order mark, its fields separated by a tab.
    Tsv,
    /// An Excel workbook, one sheet.
    Xlsx,
    /// The file the engine writes itself.
    Parquet,
}

impl DownloadFormat {
    /// The parameter value (`csv`, `tsv`, `xlsx`, `parquet`).
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "csv" => Some(Self::Csv),
            "tsv" => Some(Self::Tsv),
            "xlsx" => Some(Self::Xlsx),
            "parquet" => Some(Self::Parquet),
            _ => None,
        }
    }

    /// The file name's extension.
    const fn extension(self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Xlsx => "xlsx",
            Self::Parquet => "parquet",
        }
    }

    /// The response's `Content-Type`.
    fn content_type(self) -> axum::http::HeaderValue {
        match self {
            Self::Csv => axum::http::HeaderValue::from_static("text/csv; charset=utf-8"),
            Self::Tsv => {
                axum::http::HeaderValue::from_static("text/tab-separated-values; charset=utf-8")
            }
            Self::Xlsx => axum::http::HeaderValue::from_static(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            Self::Parquet => axum::http::HeaderValue::from_static("application/octet-stream"),
        }
    }

    /// The writer of this format when this service writes it: the three
    /// formats built from rows. `None` for the file the engine writes.
    fn row_format(self) -> Option<crate::export::FileFormat> {
        match self {
            Self::Csv => Some(crate::export::FileFormat::Csv),
            Self::Tsv => Some(crate::export::FileFormat::Tsv),
            Self::Xlsx => Some(crate::export::FileFormat::Xlsx),
            Self::Parquet => None,
        }
    }

    /// The most rows the file may hold: the workbook's own sheet limit, and
    /// no limit for the delimited files.
    fn row_cap(self) -> usize {
        match self {
            Self::Xlsx => crate::export::xlsx_row_cap(usize::MAX),
            _ => usize::MAX,
        }
    }
}

/// Which limit cut a file short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CutBy {
    /// The file holds as many rows as the format allows.
    Rows,
    /// The file holds as many bytes as the cap allows.
    Bytes,
}

impl CutBy {
    /// The header's value.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Rows => "rows",
            Self::Bytes => "bytes",
        }
    }
}

/// Which limit cut a file that was cut short: the rows the format allows, or
/// the bytes the cap allows.
///
/// A file that holds as many rows as its format could ever hold was cut by
/// the row limit; anything else was cut by the byte cap.
fn cut_by(format: DownloadFormat, rows: usize) -> CutBy {
    if rows == format.row_cap() {
        CutBy::Rows
    } else {
        CutBy::Bytes
    }
}

/// The most bytes a download may hold: 5 GB (`QS-2`).
const DOWNLOAD_BYTE_CAP: u64 = 5 * 1024 * 1024 * 1024;

/// What a result over the cap is told (`QS-2`). The file is not sent, not
/// even half of it, so this sentence is the whole answer: nothing of the
/// engine's own text is in it.
const DOWNLOAD_TOO_LARGE: &str =
    "This result is larger than 5 GB. Add a filter or download it as CSV.";

/// What a query string the download route cannot read is told.
///
/// The twin of this sentence lives in `routes/uploads.rs`
/// (`QUERY_UNREADABLE`), and this file's history route has its own copy of
/// it (`HISTORY_QUERY_UNREADABLE`). The three cannot be one constant yet:
/// the place they could share is `routes/mod.rs`, and that file is held by
/// another open pull request.
const DOWNLOAD_QUERY_UNREADABLE: &str = "The query string could not be read.";

/// The name the download's query carries on the engine: the caller's own id,
/// the run, and what the query is. Stop's pattern
/// (`<caller uuid>:<runId>:%`) finds it, so Stop reaches the download's
/// query on the engine (`QS-2`).
fn download_query_id(caller: Uuid, run_id: Uuid) -> String {
    format!("{caller}:{run_id}:download")
}

/// The file's name: the run's id, and how many rows it holds when it was cut
/// short.
fn download_file_name(id: &str, format: DownloadFormat, cut_rows: Option<usize>) -> String {
    match cut_rows {
        Some(rows) => format!("{id}-first-{rows}-rows.{}", format.extension()),
        None => format!("{}.{}", id, format.extension()),
    }
}

/// `query_history.id` shape: the old `q-<epoch_ms>`, which every row
/// recorded so far carries, and the new `q-<epoch_ms>-<32 lowercase hex>`.
/// Validated before `id` is ever interpolated into the
/// `Content-Disposition` header (WS2 plan review W9) — this is a
/// header-injection guard, not a lookup optimization; the database lookup
/// itself already binds `id` as a parameter and needs no such check.
fn is_valid_history_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("q-") else {
        return false;
    };
    // The old shape: the epoch milliseconds and nothing else.
    if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    // The new shape: the epoch milliseconds, a `-`, and the uuid's simple
    // form — 32 lowercase hex characters, and nothing else.
    let Some((epoch, suffix)) = rest.rsplit_once('-') else {
        return false;
    };
    !epoch.is_empty()
        && epoch.bytes().all(|b| b.is_ascii_digit())
        && suffix.len() == 32
        && suffix
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Only these `query_history.engine` values may be re-run through this
/// route (WS2 plan review W9). `"hot-store"` is the legacy placeholder
/// every row recorded before this workstream threaded a real `engine`
/// field carries — it named `ClickHouse`'s storage tier, not a different
/// engine, so it is downloadable exactly like `"clickhouse"`. `"trino"`
/// is deliberately excluded: re-running it on the wrong engine is not an
/// option this route offers.
fn is_downloadable_engine(engine: &str) -> bool {
    engine == "clickhouse" || engine == "hot-store"
}

/// The statement the download runs: the caller's own statement, wrapped in a
/// subquery.
///
/// The wrapper keeps the statement — comment and all — fully enclosed, so a
/// trailing `-- comment` cannot reach the `FORMAT` clause this route
/// appends and turn the file into the engine's default format. It is the
/// same reason the route wrapped the statement when it appended a `LIMIT`.
fn download_statement(sql: &str) -> String {
    let trimmed = lakehouse_clickhouse::strip_trailing_semicolon(sql.trim_end());
    format!("SELECT * FROM (\n{trimmed}\n)")
}

/// Write the rows of a download as the file `format` says.
///
/// The rows come from the engine as they arrive and are written to a file on
/// disk, so a big result never sits in memory. The byte cap cuts the file at
/// the last whole row that fits: a row that would pass the cap is not
/// written, not half written.
///
/// # Errors
///
/// [`crate::export::ExportError`]: the engine failed, the disk failed, or
/// the workbook writer refused a cell.
async fn write_download_rows<S: crate::export::RowSource>(
    source: &mut S,
    format: crate::export::FileFormat,
    locale: &str,
    byte_cap: u64,
) -> Result<crate::export::Written, crate::export::ExportError> {
    // A Query Studio result has no chart, so a column has no label of its own
    // and no value format: the header holds the column names and a number is
    // the number the engine sent.
    let labels: HashMap<String, String> = HashMap::new();
    let formats: HashMap<String, lakehouse_bi::style::ValueFormat> = HashMap::new();
    let options = crate::export::ExportOptions {
        format,
        values: crate::export::Values::Raw,
        // No row cap of this route's own: the download holds the whole
        // result. The workbook's own sheet limit still applies (the writer
        // applies it).
        cap: usize::MAX,
        locale,
        labels: &labels,
        formats: &formats,
    };
    crate::export::write_export_with_byte_cap(source, &options, Some(byte_cap)).await
}

/// The file the engine wrote itself, or the refusal when it passes the byte
/// cap.
///
/// The file cannot be cut at a row the way a file of rows can, so a result
/// over the cap is refused whole: half a table is not a file anyone can use.
/// The cap is a parameter so the refusal is provable without five gigabytes
/// of data; the route passes the cap the signed limits name.
///
/// # Errors
///
/// 422 [`ApiError::Unprocessable`] with the fixed sentence of
/// [`DOWNLOAD_TOO_LARGE`] when the file passes the cap — this code base's own
/// message, so it carries no reference — and the file's own failure,
/// reported under a reference.
async fn download_engine_file<S: crate::export::ByteSource>(
    source: &mut S,
    byte_cap: u64,
) -> Result<crate::export::ByteFile, ApiError> {
    match crate::export::write_bytes_file(source, Some(byte_cap)).await {
        Ok(Some(file)) => Ok(file),
        Ok(None) => Err(ApiError::Unprocessable(DOWNLOAD_TOO_LARGE.to_owned())),
        Err(err) => Err(download_failure(&err)),
    }
}

/// A failure of the file itself, reported under a reference with a fixed
/// sentence (`SEC-11`).
///
/// The dashboard's own export has the same mapping in
/// `routes::dashboard::export_failure`, which is private to that module.
fn download_failure(err: &crate::export::ExportError) -> ApiError {
    match err {
        crate::export::ExportError::Source(ch) => {
            upstream_error::ch_error(&upstream_error::DATABASE, ch)
        }
        other => upstream_error::internal_error(&upstream_error::SERVICE, other),
    }
}

/// A header value from text this code wrote.
///
/// The values are a file name of digits, a count and a format — printable
/// ASCII this code wrote — so a value the header cannot hold is a bug here,
/// not something the caller sent.
fn download_header(text: &str) -> Result<axum::http::HeaderValue, ApiError> {
    axum::http::HeaderValue::from_str(text)
        .map_err(|_| ApiError::Internal("a download header could not be built".to_owned()))
}

/// The download's response: the file streamed from disk, with the counts the
/// console tells the person (`X-Export-Rows`, `X-Export-Cut`,
/// `X-Export-Cut-By`).
///
/// The body is a file, not a JSON value, so it bypasses `ApiJson`.
fn download_response(
    file: tokio::fs::File,
    len: u64,
    file_name: &str,
    format: DownloadFormat,
    rows: Option<usize>,
    cut: bool,
    cut_by: Option<CutBy>,
) -> Result<Response, ApiError> {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, format.content_type());
    headers.insert(
        header::CONTENT_DISPOSITION,
        download_header(&format!("attachment; filename=\"{file_name}\""))?,
    );
    headers.insert(header::CONTENT_LENGTH, download_header(&len.to_string())?);
    headers.insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-export-cut"),
        axum::http::HeaderValue::from_static(if cut { "true" } else { "false" }),
    );
    // The row count is the row path's own: the file the engine writes was
    // never seen row by row here, so no count is claimed for it.
    if let Some(rows) = rows {
        headers.insert(
            axum::http::HeaderName::from_static("x-export-rows"),
            download_header(&rows.to_string())?,
        );
    }
    // A file that was not cut names no limit.
    if let Some(cut_by) = cut_by {
        headers.insert(
            axum::http::HeaderName::from_static("x-export-cut-by"),
            download_header(cut_by.as_str())?,
        );
    }
    let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(file));
    Ok((headers, body).into_response())
}

/// `GET /api/query/run/{id}/download?format=csv|tsv|xlsx|parquet` — re-run a
/// past `query_history` row's `SQL` against current data and return the
/// result as a file download (`QS-2`).
///
/// This is NOT the original result the caller once saw: it re-executes
/// the stored statement against whatever `ClickHouse` holds right now.
/// Data ingested or deleted since the original run changes what comes back.
/// The statement is rewritten for the caller's current roles first, exactly
/// as the run route does.
///
/// The result is the whole one: the row cap of ten thousand rows this route
/// used to apply is gone (`QS-2`). What bounds a download now is the byte cap
/// ([`DOWNLOAD_BYTE_CAP`], 5 GB) and, for the workbook, the sheet's own
/// 1,048,575 data rows. A file that passes the byte cap is cut at the last
/// whole row that fits and says so (`X-Export-Cut`, `X-Export-Cut-By`); the
/// file the engine writes itself (`Parquet`) cannot be cut at a row, so a
/// result over the cap is refused whole ([`DOWNLOAD_TOO_LARGE`]).
///
/// The download runs under the caller's own time limit, and its query is
/// named after the caller and the run (`<caller uuid>:<runId>:download`), so
/// Stop's pattern (`<caller uuid>:<runId>:%`) reaches it on the engine.
///
/// The stored statement is the one the user wrote ([`run`] records it, not
/// the rewritten form), so today's policy is applied to it once. A row
/// recorded before that change holds the rewritten form: an old mask or row
/// filter sits inside it and today's policy applies on top, which only adds
/// restriction (it fails closed). Those rows are left as they are (`SEC-21`,
/// `SEC-33`).
///
/// Only rows whose `engine` is `"clickhouse"` or the legacy `"hot-store"`
/// placeholder are eligible (WS2 plan review W9) — a `"trino"` row cannot
/// be re-run through this route at all.
///
/// Ownership is exact-match on `query_history.owner_id`
/// (`0046_query_ownership.sql`) against the caller's own principal id.
/// Every row recorded before that migration — and every row recorded with
/// no authenticated caller — has `owner_id = NULL`, which never equals any
/// principal's id, so those legacy rows are permanently undownloadable
/// through this route. That is a deliberate, fail-closed consequence of
/// scoping by identity, not an oversight to be worked around.
///
/// A role that is switched off from downloading may not download here either
/// (`BI-1` D7), and nothing is read from the engine for them.
///
/// # Errors
///
/// - 400 [`ApiError::BadRequest`] when `format` is none of `csv`, `tsv`,
///   `xlsx` and `parquet`, when `runId` is not a UUID, when `id` does not
///   match the `query_history.id` shape (`q-<digits>` or
///   `q-<digits>-<32 lowercase hex>`), or when the query string cannot be
///   read at all — a repeated parameter, or no `format` — with the fixed
///   sentence of [`DOWNLOAD_QUERY_UNREADABLE`].
/// - 403 [`ApiError::PermissionDenied`] when every role of the caller is
///   switched off from downloading.
/// - 404 [`ApiError::NotFound`] when no history row matches `id`, or when
///   one does but its recorded owner is not the calling principal
///   (including every legacy `"anonymous"` row — see this doc comment).
/// - 422 [`ApiError::Unprocessable`] when the policy rewrite refuses the
///   statement (the run route's fixed message), when the statement names a
///   setting other than `iceberg_snapshot_id` (`SEC-21`), when the row's
///   `engine` is not downloadable, when its stored `SQL` fails [`is_read_only`]
///   (defense in depth: `routes::query::run` already gated this at insert
///   time), when the engine rejects the re-run, or when the result is over
///   the byte cap.
/// - 503 [`ApiError::Unavailable`] when the query store or `ClickHouse`
///   itself is unreachable.
pub async fn download(
    State(state): State<AppState>,
    Path(id): Path<String>,
    query: Result<Query<DownloadQuery>, axum::extract::rejection::QueryRejection>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    // A query string the extractor cannot read — a repeated parameter, or no
    // `format` at all — is refused with this route's own sentence. The
    // rejection's own text is serde's, and it stays in the log: a response
    // never carries a library's rendering of the caller's request.
    let Query(query) = query.map_err(|err| {
        tracing::warn!(%err, "a download query string could not be read");
        ApiError::BadRequest(DOWNLOAD_QUERY_UNREADABLE.to_owned())
    })?;
    let Some(format) = DownloadFormat::parse(&query.format) else {
        return Err(
            ApiError::BadRequest("format must be csv, tsv, xlsx or parquet".to_owned()).into(),
        );
    };
    // The run the download belongs to. Stop names the queries of one run by
    // their name, so the download's own query must carry the same run id;
    // a caller that names none gets a run of its own.
    let run_id = match query.run_id.as_deref() {
        Some(raw) => Uuid::parse_str(raw.trim())
            .map_err(|_| ApiError::BadRequest("runId must be a UUID".to_owned()))?,
        None => Uuid::new_v4(),
    };
    if !is_valid_history_id(&id) {
        return Err(ApiError::BadRequest("id must look like a query run id".to_owned()).into());
    }

    let Some(item) = queries::get_history_item(pool(&state)?, &id).await? else {
        return Err(ApiError::NotFound("query run not found".to_owned()).into());
    };

    // Fails closed identically for a stranger's row and a legacy
    // pre-`0046` row with no recorded `owner_id` (see this function's doc
    // comment) — a caller cannot distinguish "not yours" from "does not
    // exist" either way. Checked against `owner_id`
    // (`0046_query_ownership.sql`), not the `user` display name: `user` no
    // longer carries a principal id (`QueryHistoryItem::user`'s own doc
    // comment).
    let caller_id = principal.as_ref().map(|Extension(p)| p.id.uuid());
    if caller_id.is_none() || caller_id != item.owner_id {
        return Err(ApiError::NotFound("query run not found".to_owned()).into());
    }

    if !is_downloadable_engine(&item.engine) {
        return Err(
            ApiError::Unprocessable("download supports ClickHouse runs only".to_owned()).into(),
        );
    }
    // Defense in depth: `routes::query::run` already refused a non-read-only
    // statement at insert time, so this should be unreachable in practice —
    // re-checked here because this route is about to re-execute the stored
    // SQL, not merely read it back.
    if !is_read_only(&item.sql) {
        return Err(ApiError::Unprocessable("stored query is not read-only".to_owned()).into());
    }

    // Review fix: the download applies the same policy rewrite as the run
    // (same function, the caller's current roles), so a mask or row filter that
    // governs the caller today governs the file. A refusal by the rewrite is a
    // refusal of the download, with the run route's status and fixed message;
    // nothing is sent to the engine then. The caller is present: the ownership
    // check above already refused an absent principal.
    let Some(Extension(caller)) = principal.as_ref() else {
        return Err(ApiError::NotFound("query run not found".to_owned()).into());
    };
    // The role's own download switch (`BI-1` D7): a role switched off from
    // downloading may not download here either, and nothing is read from the
    // engine for them.
    crate::routes::settings::require_download(&state, caller).await?;
    let rewritten =
        rewrite_sql_for_principal_with_headers(&state, &item.sql, &item.engine, caller, &headers)
            .await?;
    // SEC-21: the stored statement may carry a setting recorded before the
    // rule. The statement this route sends afterwards is not subject to it.
    refuse_unlisted_settings(&rewritten)?;

    // The caller's own time limit, and the name Stop finds this query by.
    let limits = crate::query_limits::QueryRun::current_or_detached()
        .limits()
        .await?;
    let options = lakehouse_clickhouse::QueryOptions {
        max_execution_time: Some(limits.timeout),
        // No row cap: the download holds the whole result.
        max_result_rows: None,
        query_id: Some(download_query_id(caller.id.uuid(), run_id)),
    };
    let statement = download_statement(&rewritten);

    let Some(row_format) = format.row_format() else {
        // The file the engine writes itself. Its bytes are counted as they
        // arrive, and a result over the cap is refused whole: a file that
        // holds half of a table is not a file anyone can use.
        let sql = format!("{statement}\nFORMAT Parquet");
        let mut bytes = state
            .clickhouse
            .byte_stream(&sql, &options)
            .await
            .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
        let file = download_engine_file(&mut bytes, DOWNLOAD_BYTE_CAP).await?;
        let file_name = download_file_name(&id, format, None);
        return download_response(file.file, file.len, &file_name, format, None, false, None)
            .map_err(Into::into);
    };

    // The row path: the rows arrive one at a time and are written to a file
    // on disk, so a big result never sits in memory.
    let locale = crate::routes::settings::format_settings(&state)
        .await?
        .number_locale;
    let mut rows = state
        .clickhouse
        .query_stream_with_options(&statement, limits.timeout, &options)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let written = write_download_rows(&mut rows, row_format, &locale, DOWNLOAD_BYTE_CAP)
        .await
        .map_err(|err| download_failure(&err))?;
    // Which limit cut the file: the rows the format allows, or the bytes the
    // cap allows. A file that was not cut names no limit.
    let cut_by = written.cut.then(|| cut_by(format, written.rows));
    let file_name = download_file_name(&id, format, written.cut.then_some(written.rows));
    download_response(
        written.file,
        written.len,
        &file_name,
        format,
        Some(written.rows),
        written.cut,
        cut_by,
    )
    .map_err(Into::into)
}

// ── GET /api/query/scheduling (WS2 §13, WS2 plan review W10, round 2, item 1) ──

/// `{ "supported": false, "reason": ... }` — scheduled execution of a saved
/// query needs a safe per-principal authority model that does not exist
/// yet: a scheduled job has no live session to run "as," and running the
/// query under a shared service identity would execute with different
/// authority than the query's author holds at trigger time. That model is
/// planned for WS7's policy-obligations engine, not this workstream.
///
/// An earlier revision of this capability added a `saved_query.schedule_cron`
/// column that the API accepted and stored while nothing ever ran it — dead
/// schema advertising a capability that does not exist. This probe adds no
/// column, no migration, and no field on [`SavedQuery`]: nothing is stored
/// that nothing then reads (AGENTS.md principle 2).
#[must_use]
fn scheduling_capability_body() -> Value {
    json!({
        "supported": false,
        "reason": "scheduled execution needs a safe per-principal authority model, planned for WS7",
    })
}

/// `GET /api/query/scheduling` — a static capability probe the
/// saved-queries UI calls once, so it can render an honest "not supported
/// yet" instead of offering a schedule control that does nothing (WS2 §13,
/// WS2 plan review W10, round 2, item 1).
///
/// # Errors
///
/// This handler never fails — it reports a fixed, static capability.
pub async fn scheduling(State(_state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    Ok(ApiJson(scheduling_capability_body()))
}

/// The SEC-21 settings rule, tested on its own. A separate module from
/// `tests` so the two do not share a hunk.
#[cfg(test)]
mod settings_rule_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use wiremock::{MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    /// Statements the parser reads as one statement and that name a setting
    /// other than `iceberg_snapshot_id`, in every place a setting can sit.
    /// (`SELECT 1 SETTINGS ..` is not here: the parser reads `SETTINGS` as
    /// the alias of `1`, so it is in [`UNREADABLE`].)
    const REFUSED_FOR_A_SETTING: &[&str] = &[
        "SELECT n FROM serving.t SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t SETTINGS max_execution_time = 0",
        "SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1, max_result_rows = 0",
        "SELECT n FROM serving.t SETTINGS max_result_rows = 0, iceberg_snapshot_id = 1",
        "SELECT n FROM serving.t SETTINGS max_result_rows = 0 FORMAT JSON",
        "SELECT n FROM serving.t SETTINGS /* a comment */ max_result_rows = 0",
        "SELECT n FROM serving.t SETTINGS \"max_result_rows\" = 0",
        // Case-sensitive: the picker writes the lower-case name.
        "SELECT n FROM serving.t SETTINGS ICEBERG_SNAPSHOT_ID = 1",
        "SELECT n FROM serving.t SETTINGS Iceberg_Snapshot_Id = 1",
        // Every place a query can sit.
        "SELECT * FROM (SELECT n FROM serving.t SETTINGS max_result_rows = 0)",
        "WITH c AS (SELECT n FROM serving.t SETTINGS max_result_rows = 0) SELECT * FROM c",
        "SELECT (SELECT n FROM serving.t SETTINGS max_result_rows = 0) AS x",
        "SELECT n FROM serving.t WHERE n IN (SELECT n FROM serving.u SETTINGS max_result_rows = 0)",
        "SELECT n FROM serving.t WHERE EXISTS (SELECT n FROM serving.u SETTINGS max_result_rows = 0)",
        "SELECT n FROM serving.t UNION ALL SELECT n FROM serving.u SETTINGS max_result_rows = 0",
        "(SELECT n FROM serving.t SETTINGS max_result_rows = 0) UNION ALL SELECT n FROM serving.u",
        "SELECT a.n FROM serving.t a JOIN (SELECT n FROM serving.u SETTINGS max_result_rows = 0) b ON a.n = b.n",
        "EXPLAIN SELECT n FROM serving.t SETTINGS max_result_rows = 0",
        "EXPLAIN SELECT * FROM (SELECT n FROM serving.t SETTINGS max_result_rows = 0)",
        "EXPLAIN ESTIMATE SELECT n FROM serving.t SETTINGS max_result_rows = 0",
        // After each clause the setting can follow.
        "SELECT n FROM serving.t FINAL SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t ARRAY JOIN arr SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t GROUP BY n WITH ROLLUP SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t LIMIT 10 OFFSET 2 SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t\n-- a comment\nSETTINGS max_result_rows = 0",
        // A setting that is not on a query.
        "SELECT * FROM executable('x.py', TabSeparated, 'a UInt8', SETTINGS pool_size = 16)",
        "INSERT INTO t SETTINGS async_insert = 1 FORMAT JSONEachRow",
    ];

    /// Statements the rule refuses because it cannot read them as exactly one
    /// statement. The engine would not see a different statement for these,
    /// and the rewrite refuses most of them first.
    const UNREADABLE: &[&str] = &[
        "SELECT 1 SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t; SELECT n FROM serving.u",
        "SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1; SELECT 2",
        "SET max_result_rows = 0; SELECT n FROM serving.t",
        "SELECT n FROM serving.t FORMAT JSON SETTINGS max_result_rows = 0",
        "SELEC n FROM serving.t",
        "SELECT n FROM serving.t SETTINGS",
        // Statement kinds that can carry no setting in the parser's tree, so
        // the parser must not accept them with one.
        "SHOW TABLES SETTINGS max_result_rows = 0",
        "DESCRIBE serving.t SETTINGS max_result_rows = 0",
        "EXPLAIN AST SELECT n FROM serving.t SETTINGS max_result_rows = 0",
        "SELECT n FROM serving.t INTO OUTFILE 'x' SETTINGS max_result_rows = 0",
        "",
    ];

    /// The picker's output shapes (`src/lib/snapshot-picker.ts`) and
    /// statements with no setting at all.
    const ALLOWED: &[&str] = &[
        "SELECT 1",
        "SELECT 1;",
        "SELECT n FROM serving.t WHERE n > 1 LIMIT 10",
        "-- a comment\nSELECT 1",
        "SELECT 'SETTINGS max_result_rows = 0' AS note",
        "SELECT 1 AS settings",
        "EXPLAIN SELECT 1",
        "SHOW TABLES",
        "DESCRIBE serving.t",
        "SELECT n FROM serving.t\nSETTINGS iceberg_snapshot_id = 1",
        "SELECT n FROM serving.t\nSETTINGS iceberg_snapshot_id = 4733245589251823498",
        "SELECT n FROM serving.t LIMIT 5\nSETTINGS iceberg_snapshot_id = 1;",
        "SELECT * FROM (SELECT n FROM serving.t) SETTINGS iceberg_snapshot_id = 1",
        "SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1 FORMAT JSON",
        "SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1, iceberg_snapshot_id = 2",
        "EXPLAIN SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1",
    ];

    fn refusal(sql: &str) -> ApiError {
        refuse_unlisted_settings(sql).expect_err(&format!("expected a refusal: {sql:?}"))
    }

    #[test]
    fn a_statement_with_any_other_setting_is_refused_with_one_fixed_message() {
        for sql in REFUSED_FOR_A_SETTING.iter().chain(UNREADABLE) {
            match refusal(sql) {
                ApiError::Unprocessable(message) => {
                    assert_eq!(message, SETTINGS_REFUSAL_MESSAGE, "{sql:?}");
                }
                other => panic!("expected 422 for {sql:?}, got {other:?}"),
            }
        }
    }

    /// Without this, a refusal above could come from the parser failing and
    /// not from the rule, and removing the rule would leave the test green.
    #[test]
    fn every_statement_refused_for_a_setting_parses_as_one_statement() {
        for sql in REFUSED_FOR_A_SETTING {
            let parsed = sqlparser::parser::Parser::parse_sql(
                &sqlparser::dialect::ClickHouseDialect {},
                sql,
            );
            assert_eq!(parsed.map(|statements| statements.len()), Ok(1), "{sql:?}");
        }
    }

    #[test]
    fn the_version_pickers_setting_and_a_statement_without_settings_pass() {
        for sql in ALLOWED {
            assert!(
                refuse_unlisted_settings(sql).is_ok(),
                "expected allowed: {sql:?}"
            );
        }
    }

    #[test]
    fn the_refusal_says_nothing_about_the_statement() {
        let message = SETTINGS_REFUSAL_MESSAGE;
        for sql in [
            "SELECT secret_column FROM salary SETTINGS salary_total = 0",
            "SELEC salary",
        ] {
            let ApiError::Unprocessable(shown) = refusal(sql) else {
                panic!("expected 422 for {sql:?}");
            };
            assert_eq!(shown, message);
            assert!(!shown.contains("salary") && !shown.contains("secret_column"));
        }
    }

    fn database_url_for(pool: &PgPool) -> String {
        let options = pool.connect_options();
        format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        )
    }

    /// The shared catalog belongs to one tenant, and the query path is
    /// scoped to the tenant: this fixture principal is the catalog's own
    /// member, so the shared tables stay reachable.
    const CATALOG_TENANT_ID: &str = "11111111-1111-4111-8111-000000000001";

    fn analyst() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000001")],
            display_name: "alice".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: vec!["Analyst".to_owned()],
        }
    }

    /// The Fix-with-AI check sends the statement to the engine inside an
    /// `EXPLAIN PLAN`, so it applies the rule too.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_proposal_check_refuses_another_setting_before_the_engine_is_asked(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [], "rows": 0,
            })))
            .mount(&server)
            .await;
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("CH_URL".to_owned(), server.uri());
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

        let refused = check_statement_would_run(
            &state,
            &analyst(),
            "SELECT n FROM serving.t SETTINGS max_result_rows = 0",
            "clickhouse",
        )
        .await;
        assert_eq!(refused, Err(SETTINGS_REFUSAL_MESSAGE.to_owned()));

        let leaked: Vec<String> = server
            .received_requests()
            .await
            .expect("mock server records requests")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| !b.contains("system."))
            .collect();
        assert!(
            leaked.is_empty(),
            "nothing may reach the engine: {leaked:?}"
        );

        let allowed = check_statement_would_run(
            &state,
            &analyst(),
            "SELECT n FROM serving.t SETTINGS iceberg_snapshot_id = 1",
            "clickhouse",
        )
        .await;
        assert_eq!(allowed, Ok(()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::{PermissionSet, PrincipalId};
    use uuid::Uuid;

    use super::*;
    use crate::sql_guard::strip_sql_noise;

    /// A logged-in human principal — `PrincipalId::User`.
    fn fixture_user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000001")],
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A service-token principal — `PrincipalId::Service`, e.g. the Dagster
    /// orchestrator running a scheduled query.
    fn fixture_service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn query_run_audit_event_uses_the_real_principal_and_a_truncated_sql() {
        let principal = fixture_user_principal();
        let event = query_run_audit_event(&principal, "q-123", "SELECT * FROM orders");
        assert_eq!(event.principal_kind.as_deref(), Some("user"));
        assert_eq!(
            event.principal_id.as_deref(),
            Some(principal.id.uuid().to_string().as_str())
        );
        assert_eq!(event.resource_kind.as_deref(), Some("query_history"));
        assert_eq!(event.resource_id.as_deref(), Some("q-123"));
        assert_eq!(event.action, "query.run");
        assert_eq!(event.outcome, "executed");
    }

    #[test]
    fn query_run_audit_event_records_a_service_identity_as_service_not_user() {
        let principal = fixture_service_principal();
        let event = query_run_audit_event(&principal, "q-124", "SELECT 1");
        assert_eq!(event.principal_kind.as_deref(), Some("service"));
    }

    #[test]
    fn query_run_audit_event_truncates_long_sql() {
        // Mirrors `routes::ai::audit`'s own `redact_truncates_long_strings`
        // assertion shape (`…[truncated]` suffix, strictly shorter than the
        // input) rather than a fixed length bound — `truncate_string`
        // keeps the first `MAX_STRING_LEN` characters AND appends the
        // marker, so the output is `MAX_STRING_LEN` plus the marker's own
        // length, not `MAX_STRING_LEN` or fewer.
        let principal = fixture_user_principal();
        let long_sql = "x".repeat(600);
        let event = query_run_audit_event(&principal, "q-125", &long_sql);
        let recorded_sql = event.args.unwrap()["sql"].as_str().unwrap().to_owned();
        assert!(
            recorded_sql.ends_with("…[truncated]"),
            "must reuse ai::audit's MAX_STRING_LEN truncation, not the raw SQL: {recorded_sql:?}"
        );
        assert!(recorded_sql.chars().count() < long_sql.chars().count());
    }

    #[test]
    fn allows_each_permitted_keyword() {
        for sql in [
            "SELECT 1",
            "with x as (select 1) select * from x",
            "SHOW TABLES",
            "DESCRIBE t",
            "DESC t",
            "EXPLAIN SELECT 1",
        ] {
            assert!(is_read_only(sql), "expected allowed: {sql}");
        }
    }

    #[test]
    fn denies_each_denied_keyword_even_after_select() {
        for kw in [
            "insert", "alter", "drop", "delete", "update", "create", "truncate", "rename",
            "attach", "detach", "grant", "revoke",
        ] {
            let sql = format!("SELECT 1; {kw} something");
            assert!(!is_read_only(&sql), "expected denied: {sql}");
        }
    }

    #[test]
    fn denies_bare_dml() {
        assert!(!is_read_only("INSERT INTO x VALUES (1)"));
        assert!(!is_read_only("DROP TABLE x"));
    }

    #[test]
    fn allows_leading_whitespace() {
        assert!(is_read_only("   \n\t SELECT 1"));
    }

    #[test]
    fn allows_a_comment_before_the_statement() {
        // The editor opens with exactly this shape — a starter comment,
        // then the user's query underneath — and it used to be refused.
        assert!(is_read_only("-- Write SQL here\nSELECT 1"));
        assert!(is_read_only("/* c */ SELECT 1"));
        assert!(is_read_only("# hash comment\nSELECT 1"));
    }

    #[test]
    fn a_dml_word_inside_a_comment_or_string_is_not_dml() {
        assert!(is_read_only("SELECT 1 -- drop this later"));
        assert!(is_read_only("/* TODO: delete */ SELECT 1"));
        assert!(is_read_only("SELECT * FROM t WHERE action = 'drop'"));
        assert!(is_read_only("SELECT 'it''s an update' AS note"));
        // …but real DML beside a decoy string still is.
        assert!(!is_read_only("SELECT 'drop' AS a; DROP TABLE t"));
    }

    #[test]
    fn strip_sql_noise_blanks_comments_and_literals() {
        // Exact spacing is irrelevant to the guard — what matters is that
        // the comment is gone and the words around it are not glued
        // together.
        let stripped = strip_sql_noise("SELECT 1 -- drop\nFROM t");
        assert!(!stripped.contains("drop"), "{stripped}");
        assert!(stripped.contains("SELECT 1"), "{stripped}");
        assert!(stripped.contains("FROM t"), "{stripped}");
        assert_eq!(strip_sql_noise("SELECT /* x */ 1"), "SELECT   1");
        assert_eq!(
            strip_sql_noise("SELECT 'a' , \"b\" , `c`"),
            "SELECT   ,   ,  "
        );
        // An unterminated quote swallows the rest, which is the safe way
        // round: the guard then sees less, never more.
        assert_eq!(strip_sql_noise("SELECT 'unterminated"), "SELECT  ");
    }

    #[test]
    fn a_past_version_clause_is_found_in_the_tokens() {
        // DATA-16 F3.
        assert!(uses_past_version_clause(
            "SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 1"
        ));
        assert!(uses_past_version_clause(
            "SELECT * FROM t FOR TIMESTAMP AS OF TIMESTAMP '2026-01-01 00:00:00'"
        ));
        assert!(uses_past_version_clause(
            "select * from t version\n/* c */ as of 1"
        ));
        assert!(uses_past_version_clause("SELECT * FROM t VERSION AS OF 1"));
    }

    #[test]
    fn the_same_words_in_a_string_or_comment_are_not_a_clause() {
        assert!(!uses_past_version_clause(
            "SELECT 'FOR VERSION AS OF 1' FROM t WHERE ("
        ));
        assert!(!uses_past_version_clause(
            "SELECT 1 -- VERSION AS OF 1\nWHERE ("
        ));
        assert!(!uses_past_version_clause(
            "SELECT \"version\" AS of_x FROM t"
        ));
        assert!(!uses_past_version_clause("SELECT version, x AS of FROM t"));
        assert!(!uses_past_version_clause("SELECT FROM WHERE ("));
        // An unterminated literal fails to tokenize: today's message stays.
        assert!(!uses_past_version_clause("SELECT 'VERSION AS OF"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(is_read_only("SeLeCt 1"));
        assert!(!is_read_only("select 1; DeLeTe from t"));
    }

    #[test]
    fn smuggled_dml_after_select_via_semicolon_is_rejected() {
        assert!(!is_read_only("SELECT 1 AS a; DELETE FROM t"));
    }

    #[test]
    fn word_boundary_avoids_false_positive_substring_match() {
        // "createdAt" contains "create" but not as a whole word.
        assert!(is_read_only("SELECT createdAt FROM t"));
        // "updated_at" contains "update" but not as a whole word (the
        // trailing `_` is a \w character, so no boundary there).
        assert!(is_read_only("SELECT updated_at FROM t"));
    }

    #[test]
    fn rejects_statement_starting_with_disallowed_keyword() {
        assert!(!is_read_only("TRUNCATE t"));
        assert!(!is_read_only("GRANT SELECT ON t TO u"));
    }

    #[test]
    fn strip_trailing_semicolon_removes_one_trailing_semicolon_and_whitespace() {
        assert_eq!(strip_trailing_semicolon("SELECT 1;  "), "SELECT 1");
        assert_eq!(strip_trailing_semicolon("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn numeric_value_coerces_string_and_number() {
        assert_eq!(numeric_value(&json!(5)), Some(5.0));
        assert_eq!(numeric_value(&json!("7")), Some(7.0));
        assert_eq!(numeric_value(&json!(null)), None);
    }

    #[test]
    fn run_result_emits_null_for_unmeasured_transparency_fields() {
        let v = run_result_json(
            "q-1",
            &json!([]),
            &json!([]),
            (0, 2_000),
            None,
            12,
            34,
            1,
            "clickhouse",
        );

        // The transparency panel exists to tell the reader what actually
        // happened. ClickHouse's response carries no cache-hit flag and no
        // pushdown list, and no policy engine exists yet (WS7), so these are
        // not measurements.
        for key in ["cacheHit", "pushdowns", "policyObligations"] {
            assert!(
                v["metrics"][key].is_null(),
                "{key} must be null, got {}",
                v["metrics"][key]
            );
        }
        assert!(
            v["plan"].is_null(),
            "the plan was one invented stage, not a real plan"
        );

        // Genuinely measured values survive.
        assert_eq!(v["metrics"]["durationMs"], json!(12));
        assert_eq!(v["metrics"]["scannedBytes"], json!(34));
        assert_eq!(v["metrics"]["costUnits"], json!(1));

        // The new top-level `engine` (which engine ran) and the untouched
        // `metrics.engine` (ClickHouse's storage tier) are two different
        // concepts and must not collapse into one value.
        assert_eq!(v["engine"], json!("clickhouse"));
        assert_eq!(v["metrics"]["engine"], json!("hot-store"));
    }

    // ── The server-side byte cap (`QS-2`) ────────────────────────────────

    #[test]
    fn a_cell_costs_its_text_plus_one_byte_for_the_separator() {
        // A JSON string's own bytes, without the quotes.
        assert_eq!(cell_byte_cost(&json!("abc")), 4);
        // Any other value's compact JSON text.
        assert_eq!(cell_byte_cost(&json!(5)), 2);
        assert_eq!(cell_byte_cost(&json!(true)), 5);
        assert_eq!(cell_byte_cost(&json!(null)), 5);
    }

    #[test]
    fn a_row_costs_the_sum_of_its_cells() {
        assert_eq!(row_byte_cost(&json!({"a": "xx", "b": "yyy"})), 3 + 4);
        assert_eq!(row_byte_cost(&json!(["xx", "yyy"])), 3 + 4);
    }

    #[test]
    fn a_result_under_the_byte_cap_is_kept_whole() {
        let (kept, cut) = cut_rows_by_bytes(vec![json!({"a": "xxx"}), json!({"a": "yyy"})], 1_000);
        assert!(!cut, "nothing was cut");
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn the_byte_cap_cuts_at_the_last_whole_row_that_fits() {
        // Three rows of four bytes each; the cap holds two of them.
        let rows = vec![
            json!({"a": "xxx"}),
            json!({"a": "yyy"}),
            json!({"a": "zzz"}),
        ];
        let (kept, cut) = cut_rows_by_bytes(rows, 8);
        assert!(cut, "the third row did not fit");
        assert_eq!(kept, vec![json!({"a": "xxx"}), json!({"a": "yyy"})]);
    }

    #[test]
    fn a_first_row_over_the_byte_cap_is_kept_anyway() {
        // The byte cap never empties a result: one enormous row is still an
        // answer.
        let rows = vec![json!({"a": "x".repeat(100)}), json!({"a": "y"})];
        let (kept, cut) = cut_rows_by_bytes(rows, 10);
        assert!(cut, "the second row did not fit");
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn a_run_result_without_a_cut_carries_no_cut_by_key() {
        let v = run_result_json(
            "q-1",
            &json!([]),
            &json!([]),
            (0, 2_000),
            None,
            12,
            34,
            1,
            "clickhouse",
        );
        // The key is absent when nothing was cut — not `null`, which would
        // claim a cut that did not happen.
        assert!(v.get("cutBy").is_none(), "no cut, no key: {v}");
        assert_eq!(v["truncated"], json!(false));
    }

    #[test]
    fn a_run_result_names_the_limit_that_cut_it() {
        let rows = json!([{"a": "x"}]);
        let by_rows = run_result_json(
            "q-2",
            &json!([]),
            &rows,
            (2, 1),
            Some("rows"),
            12,
            34,
            1,
            "clickhouse",
        );
        assert_eq!(by_rows["cutBy"], json!("rows"));
        assert_eq!(by_rows["truncated"], json!(true));

        let by_bytes = run_result_json(
            "q-3",
            &json!([]),
            &rows,
            (2, 1),
            Some("bytes"),
            12,
            34,
            1,
            "clickhouse",
        );
        assert_eq!(by_bytes["cutBy"], json!("bytes"));
        assert_eq!(by_bytes["truncated"], json!(true));
    }

    #[test]
    fn estimate_result_emits_null_for_unmeasured_fields() {
        let v = estimate_result_json(Some(100), Some(1.0), Some(2.0), &json!([]), None, None);

        for key in [
            "cacheEligible",
            "freshnessLagSeconds",
            "policyObligations",
            "plan",
        ] {
            assert!(v[key].is_null(), "{key} must be null, got {}", v[key]);
        }
        // The EXPLAIN ESTIMATE numbers are real.
        assert_eq!(v["estimatedBytes"], json!(100));
    }

    // ── DATA-12 part 3: marks on the sources of an estimate ─────────────

    mod source_marks {
        use std::collections::HashMap;

        // Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
        // testcontainer bootstrap runs for this test binary.
        use lakehouse_test_support as _;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        fn state_for(pool: &lakehouse_store::PgPool, ch_url: &str) -> AppState {
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
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            env.insert(
                "CATALOG_TENANT_ID".to_owned(),
                "11111111-1111-4111-8111-000000000001".to_owned(),
            );
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        /// A `ClickHouse` whose `EXPLAIN ESTIMATE` reads one table.
        async fn clickhouse_reading(database: &str, table: &str) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [],
                    "data": [{ "database": database, "table": table, "rows": "10" }],
                    "rows": 1,
                })))
                .mount(&server)
                .await;
            server
        }

        async fn estimate_of(state: AppState) -> Value {
            estimate_of_caller(state, fixture_user_principal(), HeaderMap::new()).await
        }

        async fn estimate_of_caller(
            state: AppState,
            principal: Principal,
            headers: HeaderMap,
        ) -> Value {
            estimate_of_caller_sql(state, principal, headers, "SELECT * FROM t").await
        }

        async fn estimate_of_caller_sql(
            state: AppState,
            principal: Principal,
            headers: HeaderMap,
            sql: &str,
        ) -> Value {
            let result = estimate(
                State(state),
                Extension(principal),
                headers,
                Bytes::from(serde_json::to_vec(&json!({ "sql": sql })).unwrap()),
            )
            .await;
            let Ok(ApiJson(body)) = result else {
                panic!("an estimate never errors back to the caller");
            };
            body
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_query_reading_a_deprecated_table_marks_that_source(
            pool: lakehouse_store::PgPool,
        ) {
            lakehouse_store::annotation::set_certification(
                &pool,
                &lakehouse_store::annotation::CertificationInput {
                    asset_id: "silver.orders".to_owned(),
                    status: "deprecated".to_owned(),
                    note: Some("Use silver.orders_v2".to_owned()),
                    replacement_asset_id: Some("silver.orders_v2".to_owned()),
                    certified_by: "Rina".to_owned(),
                },
            )
            .await
            .unwrap();
            let ch = clickhouse_reading("silver", "orders").await;

            let body = estimate_of(state_for(&pool, &ch.uri())).await;

            assert_eq!(
                body["sources"],
                json!(["silver.orders"]),
                "sources stay names"
            );
            let mark = &body["sourceMarks"]["silver.orders"];
            assert_eq!(mark["certification"], json!("deprecated"));
            assert_eq!(mark["certificationNote"], json!("Use silver.orders_v2"));
            assert_eq!(mark["replacementAssetId"], json!("silver.orders_v2"));
        }

        // PR #106 review R1 BLOCKER 2
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_query_reading_a_table_with_tags_carries_them_in_source_marks(
            pool: lakehouse_store::PgPool,
        ) {
            lakehouse_store::annotation::upsert_annotation(
                &pool,
                &lakehouse_store::annotation::AnnotationInput {
                    asset_id: "silver.orders".to_owned(),
                    owner: None,
                    steward: None,
                    description: None,
                    tags: vec!["pii".to_owned()],
                },
            )
            .await
            .unwrap();
            lakehouse_store::governed_tag::create_key(
                &pool,
                "data_domain",
                &["finance".to_owned()],
                "admin",
            )
            .await
            .unwrap();
            lakehouse_store::governed_tag::set_asset_tags(
                &pool,
                "silver.orders",
                &[("data_domain".to_owned(), "finance".to_owned())],
                "admin",
            )
            .await
            .unwrap();

            let ch = clickhouse_reading("silver", "orders").await;
            let body = estimate_of(state_for(&pool, &ch.uri())).await;
            let mark = &body["sourceMarks"]["silver.orders"];
            assert_eq!(mark["tags"], json!(["pii"]));
            assert_eq!(mark["governedTags"], json!({ "data_domain": "finance" }));
        }

        // PR #106 review R1 BLOCKER 2, SHOULD-FIX DATA-12 2
        //
        // `SEC-20` moved the shared table out of this caller's reach: a
        // caller outside the catalog's tenant is refused a shared table
        // before the estimate asks the engine anything (the refusal has
        // its own test below). The DATA-12 rule — a caller outside the
        // catalog's tenant gets no source marks — is still what happens
        // for the tables that caller MAY read: its own tenant's upload.
        // The mark is seeded on that upload, so the assertion below is
        // "the mark exists and is still withheld", not "there was
        // nothing to withhold".
        #[sqlx::test(migrations = "../../migrations")]
        async fn caller_outside_catalog_tenant_receives_no_source_marks(
            pool: lakehouse_store::PgPool,
        ) {
            lakehouse_store::annotation::set_certification(
                &pool,
                &lakehouse_store::annotation::CertificationInput {
                    asset_id: "bronze.retail_orders".to_owned(),
                    status: "certified".to_owned(),
                    note: None,
                    replacement_asset_id: None,
                    certified_by: "Rina".to_owned(),
                },
            )
            .await
            .unwrap();
            // The retail tenant holds this upload, so the caller below may
            // read it: a tenant reads its own upload, whoever owns the
            // shared catalog.
            lakehouse_store::uploads::claim_table(
                &pool,
                uuid::uuid!("11111111-1111-4111-8111-000000000002"),
                "retail_orders",
                "upload-retail-orders",
            )
            .await
            .unwrap();

            let ch = clickhouse_reading("bronze", "retail_orders").await;
            let state = state_for(&pool, &ch.uri());

            // A caller who belongs to retail (not meridian-group catalog tenant):
            let outside_caller = Principal {
                id: PrincipalId::User(Uuid::new_v4()),
                tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000002")],
                display_name: "Outsider".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            };

            let body = estimate_of_caller_sql(
                state,
                outside_caller,
                HeaderMap::new(),
                "SELECT * FROM bronze.retail_orders",
            )
            .await;
            assert_eq!(body["sources"], json!(["bronze.retail_orders"]));
            assert!(
                body.get("sourceMarks").is_none(),
                "marks must be omitted for caller outside catalog tenant"
            );
        }

        // SEC-20: a caller outside the catalog's tenant is refused a shared
        // table with the fixed 404 sentence, and the engine is never asked.
        #[sqlx::test(migrations = "../../migrations")]
        async fn caller_outside_catalog_tenant_is_refused_a_shared_table(
            pool: lakehouse_store::PgPool,
        ) {
            let ch = clickhouse_reading("silver", "orders").await;
            let state = state_for(&pool, &ch.uri());

            let outside_caller = Principal {
                id: PrincipalId::User(Uuid::new_v4()),
                tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000002")],
                display_name: "Outsider".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            };

            let result = estimate(
                State(state),
                Extension(outside_caller),
                HeaderMap::new(),
                Bytes::from_static(br#"{"sql":"SELECT * FROM silver.orders"}"#),
            )
            .await;
            let Err(err) = result else {
                panic!("expected the shared table to be refused");
            };
            assert_eq!(err.0.status(), 404);
            assert_eq!(
                err.0.to_string(),
                "Table not found or not available to your organisation: silver.orders"
            );
            assert!(
                ch.received_requests()
                    .await
                    .expect("the mock records requests")
                    .is_empty(),
                "nothing may reach the engine"
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_query_reading_only_unmarked_tables_has_no_source_marks(
            pool: lakehouse_store::PgPool,
        ) {
            let ch = clickhouse_reading("silver", "plain").await;
            let body = estimate_of(state_for(&pool, &ch.uri())).await;
            assert_eq!(body["sources"], json!(["silver.plain"]));
            assert!(body.get("sourceMarks").is_none());
        }
    }

    // ── `engine: "trino"` (WS2 §4) ──────────────────────────────────────

    mod trino_engine {
        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        use uuid::Uuid;
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        /// Builds `AppState` the way `routes::connectors::state_without_pool`
        /// does (`AppState::new(Config::from_map(..))`), overriding
        /// `TRINO_URL` so `AppState::trino` points at the wiremock server
        /// instead of the real compose service. No live Postgres either
        /// (`DATABASE_URL` is deliberately unparseable): history recording
        /// degrades to a logged warning, per `record_history`'s doc
        /// comment, which is fine for these handler-level tests.
        fn test_state_with_trino(trino_uri: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert("TRINO_URL".to_owned(), trino_uri.to_owned());
            AppState::new(Config::from_map(&env).expect("valid config from a plain map"))
        }

        /// An authenticated `query:read` principal for `run`'s tests.
        /// `display_name` is deliberately a parameter — one test sets it to
        /// `"trino-maintenance"` to prove [`run`] never derives
        /// `X-Trino-User` from it.
        fn principal_with_display_name(display_name: &str) -> Extension<Principal> {
            Extension(Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: Vec::new(),
                display_name: display_name.to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            })
        }

        fn trino_body(sql: &str) -> Bytes {
            Bytes::from(serde_json::to_vec(&json!({"sql": sql, "engine": "trino"})).unwrap())
        }

        async fn assert_refused_before_reaching_trino(sql: &str) {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let result = run(
                State(state),
                Some(principal_with_display_name("alice")),
                HeaderMap::new(),
                trino_body(sql),
            )
            .await;
            assert!(result.is_err(), "expected {sql:?} to be refused");
            assert_eq!(
                server
                    .received_requests()
                    .await
                    .expect("mock server records requests")
                    .len(),
                0,
                "{sql:?} must never reach Trino"
            );
        }

        // Refusals `is_read_only` already catches, unchanged — the Trino
        // guard is additive, never a relaxation of the existing gate.
        #[tokio::test]
        async fn refuses_call_without_reaching_trino() {
            assert_refused_before_reaching_trino("CALL system.runtime.kill_query(query_id => 'x')")
                .await;
        }

        #[tokio::test]
        async fn refuses_alter_table_execute_optimize() {
            assert_refused_before_reaching_trino("ALTER TABLE t EXECUTE optimize").await;
        }

        #[tokio::test]
        async fn refuses_merge_into() {
            assert_refused_before_reaching_trino(
                "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE",
            )
            .await;
        }

        #[tokio::test]
        async fn refuses_set_session() {
            assert_refused_before_reaching_trino("SET SESSION query_max_run_time = '1h'").await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_insert() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE INSERT INTO t SELECT 1").await;
        }

        // Refusals the new Trino-specific guard adds — each of these passes
        // `is_read_only` today (verified against this branch's unmodified
        // `is_read_only`/`contains_denied_keyword` before `is_trino_safe`
        // existed): none of the 12 ClickHouse-shaped denied words appear,
        // and each starts with the allowed `explain` keyword.
        #[tokio::test]
        async fn refuses_explain_analyze_merge_bypass() {
            assert_refused_before_reaching_trino(
                "EXPLAIN ANALYZE MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN DELETE",
            )
            .await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_refresh_materialized_view_bypass() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE REFRESH MATERIALIZED VIEW v")
                .await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_comment_bypass() {
            assert_refused_before_reaching_trino("EXPLAIN ANALYZE COMMENT ON TABLE t IS 'x'").await;
        }

        #[tokio::test]
        async fn refuses_explain_analyze_with_extra_whitespace_and_mixed_case() {
            assert_refused_before_reaching_trino("ExPlAiN     aNaLyZe SELECT 1").await;
        }

        #[tokio::test]
        async fn runs_a_select_and_reports_engine_in_the_response() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/statement"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "q1",
                    "columns": [{"name": "n", "type": "bigint"}],
                    "data": [[1]],
                })))
                .mount(&server)
                .await;

            let state = test_state_with_trino(&server.uri());
            let ApiJson(value) = run(
                State(state),
                Some(principal_with_display_name("alice")),
                HeaderMap::new(),
                trino_body("SELECT 1"),
            )
            .await
            .expect("ok");

            assert_eq!(value["engine"], json!("trino"));
            assert_eq!(value["rows"], json!([{"n": "1"}]));
        }

        #[tokio::test]
        async fn x_trino_user_header_is_derived_from_the_uuid_never_the_display_name() {
            let server = MockServer::start().await;
            let expected_header = format!("console-{}", Uuid::nil());
            Mock::given(method("POST"))
                .and(path("/v1/statement"))
                .and(header("X-Trino-User", expected_header.as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "q1",
                    "columns": [{"name": "n", "type": "bigint"}],
                    "data": [[1]],
                })))
                .mount(&server)
                .await;

            let state = test_state_with_trino(&server.uri());
            // The display name is deliberately the exact string the
            // maintenance user is named — proving `X-Trino-User` is never
            // derived from it is the point of this test.
            let result = run(
                State(state),
                Some(principal_with_display_name("trino-maintenance")),
                HeaderMap::new(),
                trino_body("SELECT 1"),
            )
            .await;

            assert!(
                result.is_ok(),
                "the wiremock header matcher would reject a wrong header"
            );
        }

        #[tokio::test]
        async fn missing_principal_is_refused_before_reaching_trino() {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let result = run(State(state), None, HeaderMap::new(), trino_body("SELECT 1")).await;
            let err = result.expect_err("no principal must be refused");
            assert_eq!(err.0.status(), 401);
            assert_eq!(
                server
                    .received_requests()
                    .await
                    .expect("mock server records requests")
                    .len(),
                0
            );
        }

        #[test]
        fn maps_query_error_to_422_with_the_engines_diagnosis_and_a_reference() {
            // SEC-11 query-author exception: the author sees the engine's
            // message for their own statement, trimmed, with a reference.
            let err = map_trino_error(TrinoError::Query(
                "line 1:1: mismatched input 'x' at trino-1.internal:8080".to_owned(),
            ));
            assert_eq!(err.status(), 422);
            let text = err.to_string();
            assert!(text.contains("line 1:1: mismatched input"), "{text}");
            assert!(text.contains(" Reference: "), "{text}");
            assert!(!text.contains("trino-1.internal"), "{text}");
        }

        #[test]
        fn maps_timeout_to_503() {
            let err = map_trino_error(TrinoError::Timeout);
            assert_eq!(err.status(), 503);
        }

        #[test]
        fn maps_http_status_to_503() {
            let err = map_trino_error(TrinoError::HttpStatus(500));
            assert_eq!(err.status(), 503);
        }

        #[test]
        fn maps_too_many_rows_to_422_naming_the_cap() {
            let err = map_trino_error(TrinoError::TooManyRows { cap: 10_000 });
            assert_eq!(err.status(), 422);
            assert!(err.to_string().contains("10000") || err.to_string().contains("10,000"));
        }

        #[test]
        fn allows_a_plain_select_against_the_trino_guard() {
            assert!(is_trino_safe("SELECT 1"));
        }

        // ── WS2 §4: the real principal now gates every engine ──────────

        /// The default (`ClickHouse`) engine is refused with 401 too, not
        /// just `engine: "trino"` — `run`'s doc comment documents there is
        /// no honest `"anonymous"` fallback left for `query_history.user`.
        /// This must fail before either engine's client is ever called, so
        /// no `ClickHouse`/`Trino` reachability is needed for this test.
        #[tokio::test]
        async fn missing_principal_is_401_for_the_default_engine_too() {
            let server = MockServer::start().await;
            let state = test_state_with_trino(&server.uri());
            let body = Bytes::from(r#"{"sql": "SELECT 1"}"#);
            let err = run(State(state), None, HeaderMap::new(), body)
                .await
                .expect_err("no principal must be refused");
            assert_eq!(err.0.status(), 401);
        }

        /// A run with an authenticated principal records that principal's
        /// own uuid string as `query_history.user` — proven at the
        /// `lakehouse_store` layer
        /// (`record_history_carries_the_real_principal_uuid_and_engine` in
        /// `lakehouse-store/tests/queries.rs`), since this crate's handler
        /// tests run with no live Postgres pool (`record_history` degrades
        /// to a logged warning per its own doc comment, so there is
        /// nothing to assert against here). This test instead pins the
        /// pure derivation `run` performs — `p.id.uuid().to_string()` — so
        /// a future refactor of that one line is caught here even without
        /// a database.
        #[test]
        fn the_recorded_user_is_the_principals_uuid_string_not_the_display_name() {
            let Extension(p) = principal_with_display_name("trino-maintenance");
            let recorded_user = p.id.uuid().to_string();
            assert_eq!(recorded_user, Uuid::nil().to_string());
            assert_ne!(recorded_user, p.display_name);
        }
    }

    // ── GET /api/query/run/{id}/download (QS-2) ─────────────────────────

    mod download_pure {
        //! The parts of the download that decide, without an engine and
        //! without a database: the formats it offers, the name it gives the
        //! engine's query, the file's name, the statement it runs, and the
        //! two caps.
        //!
        //! The caps are parameters of the helpers that apply them, so the
        //! cases that need more data than a test may hold — a comma file over
        //! the byte cap, a file the engine wrote over the byte cap — are
        //! proven here, with a cap small enough to write out.

        use tokio::io::AsyncReadExt;

        use lakehouse_clickhouse::ChError;

        use super::*;

        /// Rows made on demand, so a test never holds the result it writes.
        struct GeneratedRows {
            names: Vec<String>,
            types: Vec<String>,
            rows: Vec<Vec<Value>>,
            next: usize,
        }

        impl crate::export::RowSource for GeneratedRows {
            fn names(&self) -> &[String] {
                &self.names
            }
            fn types(&self) -> &[String] {
                &self.types
            }
            fn next_row(
                &mut self,
            ) -> impl std::future::Future<Output = Result<Option<Vec<Value>>, ChError>> + Send
            {
                let row = self.rows.get(self.next).cloned();
                self.next += 1;
                std::future::ready(Ok(row))
            }
        }

        /// One column of whole numbers, the shape the engine sends them in.
        fn numbers(count: usize) -> GeneratedRows {
            GeneratedRows {
                names: vec!["n".to_owned()],
                types: vec!["Int64".to_owned()],
                rows: (1..=count)
                    .map(|n| vec![Value::String(n.to_string())])
                    .collect(),
                next: 0,
            }
        }

        /// Bytes made on demand, so a test never holds the file it counts.
        struct GeneratedBytes {
            chunks: Vec<Vec<u8>>,
            next: usize,
        }

        impl crate::export::ByteSource for GeneratedBytes {
            fn next_chunk(
                &mut self,
            ) -> impl std::future::Future<Output = Result<Option<Vec<u8>>, ChError>> + Send
            {
                let chunk = self.chunks.get(self.next).cloned();
                self.next += 1;
                std::future::ready(Ok(chunk))
            }
        }

        fn bytes(chunks: &[&[u8]]) -> GeneratedBytes {
            GeneratedBytes {
                chunks: chunks.iter().map(|c| c.to_vec()).collect(),
                next: 0,
            }
        }

        #[test]
        fn the_four_formats_are_offered_and_nothing_else_is() {
            assert_eq!(DownloadFormat::parse("csv"), Some(DownloadFormat::Csv));
            assert_eq!(DownloadFormat::parse("tsv"), Some(DownloadFormat::Tsv));
            assert_eq!(DownloadFormat::parse("xlsx"), Some(DownloadFormat::Xlsx));
            assert_eq!(
                DownloadFormat::parse("parquet"),
                Some(DownloadFormat::Parquet)
            );
            for refused in ["", "CSV", "pdf", "csv ", "xls", "parquet2"] {
                assert_eq!(DownloadFormat::parse(refused), None, "{refused:?}");
            }
        }

        #[test]
        fn every_format_names_its_own_file_and_its_own_kind() {
            assert_eq!(DownloadFormat::Csv.extension(), "csv");
            assert_eq!(DownloadFormat::Tsv.extension(), "tsv");
            assert_eq!(DownloadFormat::Xlsx.extension(), "xlsx");
            assert_eq!(DownloadFormat::Parquet.extension(), "parquet");
            assert_eq!(
                DownloadFormat::Csv.content_type(),
                "text/csv; charset=utf-8"
            );
            assert_eq!(
                DownloadFormat::Tsv.content_type(),
                "text/tab-separated-values; charset=utf-8"
            );
        }

        #[test]
        fn the_engine_writes_the_parquet_file_and_this_service_writes_the_other_three() {
            assert_eq!(
                DownloadFormat::Csv.row_format(),
                Some(crate::export::FileFormat::Csv)
            );
            assert_eq!(
                DownloadFormat::Tsv.row_format(),
                Some(crate::export::FileFormat::Tsv)
            );
            assert_eq!(
                DownloadFormat::Xlsx.row_format(),
                Some(crate::export::FileFormat::Xlsx)
            );
            assert_eq!(DownloadFormat::Parquet.row_format(), None);
        }

        #[test]
        fn the_workbook_has_a_row_limit_of_its_own_and_the_delimited_files_have_none() {
            assert_eq!(DownloadFormat::Xlsx.row_cap(), 1_048_575);
            assert_eq!(DownloadFormat::Csv.row_cap(), usize::MAX);
            assert_eq!(DownloadFormat::Tsv.row_cap(), usize::MAX);
        }

        #[test]
        fn the_query_is_named_after_the_caller_and_the_run_so_stop_finds_it() {
            let caller = Uuid::nil();
            let run = Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222);
            let name = download_query_id(caller, run);
            assert_eq!(name, format!("{caller}:{run}:download"));
            // The pattern the Stop route kills by.
            assert!(
                name.starts_with(&format!("{caller}:{run}:")),
                "{name} is not found by the stop pattern"
            );
        }

        #[test]
        fn a_file_that_was_not_cut_keeps_the_runs_own_name_and_a_cut_one_names_its_rows() {
            assert_eq!(
                download_file_name("q-1", DownloadFormat::Csv, None),
                "q-1.csv"
            );
            assert_eq!(
                download_file_name("q-1", DownloadFormat::Parquet, None),
                "q-1.parquet"
            );
            assert_eq!(
                download_file_name("q-1", DownloadFormat::Xlsx, Some(12)),
                "q-1-first-12-rows.xlsx"
            );
        }

        #[test]
        fn the_statement_is_wrapped_so_a_trailing_comment_cannot_reach_the_format_clause() {
            // A naive `format!("{sql} FORMAT Parquet")` would have this
            // trailing `--` comment out the appended clause, and the file
            // would come back in the engine's default format. Wrapping the
            // whole statement in its own subquery keeps the comment
            // enclosed, unable to reach the clause.
            assert_eq!(
                download_statement("SELECT * FROM t -- trailing comment"),
                "SELECT * FROM (\nSELECT * FROM t -- trailing comment\n)"
            );
        }

        #[test]
        fn the_statement_keeps_its_own_limit_clause_inside_the_wrapper() {
            assert_eq!(
                download_statement("SELECT * FROM t LIMIT 5"),
                "SELECT * FROM (\nSELECT * FROM t LIMIT 5\n)"
            );
        }

        #[test]
        fn the_statement_drops_one_trailing_semicolon_and_its_whitespace() {
            assert_eq!(
                download_statement("SELECT 1;   "),
                "SELECT * FROM (\nSELECT 1\n)"
            );
            assert_eq!(
                download_statement("SELECT 1"),
                "SELECT * FROM (\nSELECT 1\n)"
            );
        }

        #[test]
        fn a_file_that_holds_as_many_rows_as_its_format_allows_was_cut_by_the_rows() {
            assert_eq!(cut_by(DownloadFormat::Xlsx, 1_048_575), CutBy::Rows);
            assert_eq!(cut_by(DownloadFormat::Xlsx, 10), CutBy::Bytes);
            assert_eq!(cut_by(DownloadFormat::Csv, 10), CutBy::Bytes);
        }

        /// `QS-2`: the byte cap cuts a comma file at the last whole row that
        /// fits — a row that would pass the cap is not written, not half
        /// written. The cap is a parameter, so the case is proven here with a
        /// cap small enough to count: the header and three rows are fifteen
        /// bytes, the fourth would pass sixteen.
        #[tokio::test]
        async fn a_comma_file_over_the_byte_cap_is_cut_at_the_last_whole_row_that_fits() {
            let mut source = numbers(4);
            let mut written =
                write_download_rows(&mut source, crate::export::FileFormat::Csv, "en-US", 16)
                    .await
                    .unwrap();
            assert!(written.cut, "the fourth row does not fit in sixteen bytes");
            assert_eq!(written.rows, 3);
            let mut out = Vec::new();
            written.file.read_to_end(&mut out).await.unwrap();
            assert_eq!(out, "\u{feff}n\r\n1\r\n2\r\n3\r\n".as_bytes());
        }

        #[tokio::test]
        async fn a_comma_file_that_fits_the_byte_cap_is_not_cut() {
            let mut source = numbers(3);
            let written =
                write_download_rows(&mut source, crate::export::FileFormat::Csv, "en-US", 16)
                    .await
                    .unwrap();
            assert!(!written.cut);
            assert_eq!(written.rows, 3);
        }

        /// `QS-2`: the file the engine wrote cannot be cut at a row, so a
        /// result over the cap is refused whole. The refusal is this code
        /// base's own sentence, so it carries no reference.
        #[tokio::test]
        async fn a_file_the_engine_wrote_over_the_byte_cap_is_refused_whole() {
            let mut source = bytes(&[b"PAR1", b"\x00\x00\x00\x00", b"PAR1"]);
            let Err(err) = download_engine_file(&mut source, 8).await else {
                panic!("twelve bytes do not fit in a cap of eight");
            };
            assert_eq!(err.status(), 422);
            // The sentence is written out here, not read from the constant it
            // comes from: a test that compares a message with the message's
            // own source proves nothing.
            assert_eq!(
                err.to_string(),
                "This result is larger than 5 GB. Add a filter or download it as CSV."
            );
            assert!(!err.to_string().contains("Reference:"));
        }

        #[tokio::test]
        async fn a_file_the_engine_wrote_that_fits_the_byte_cap_is_kept_whole() {
            let mut source = bytes(&[b"PAR1", b"\x00\x00\x00\x00"]);
            let file = download_engine_file(&mut source, 8).await.unwrap();
            assert_eq!(file.len, 8);
        }

        /// `SEC-11`: the engine's own text never reaches a response — the
        /// caller gets a fixed sentence and a reference id.
        #[test]
        fn an_engine_failure_answers_a_fixed_sentence_with_a_reference() {
            let err = download_failure(&crate::export::ExportError::Source(ChError::Server(
                "Code: 47. Unknown identifier: nope".to_owned(),
            )));
            assert_eq!(err.status(), 422);
            let text = err.to_string();
            assert!(
                text.starts_with("The database request failed. Reference: "),
                "{text}"
            );
            assert!(!text.contains("Unknown identifier"), "{text}");
        }

        #[test]
        fn an_unreachable_engine_answers_503() {
            let err = download_failure(&crate::export::ExportError::Source(ChError::Cancelled));
            assert_eq!(err.status(), 503);
        }

        #[test]
        fn clickhouse_and_legacy_hot_store_engines_are_downloadable() {
            assert!(is_downloadable_engine("clickhouse"));
            assert!(is_downloadable_engine("hot-store"));
        }

        #[test]
        fn trino_and_unknown_engines_are_not_downloadable() {
            assert!(!is_downloadable_engine("trino"));
            assert!(!is_downloadable_engine("something-else"));
        }

        /// The id shape the download accepts: the old one, which every row
        /// recorded so far carries, and the new one. Both, because a row
        /// recorded before the change must stay downloadable.
        #[test]
        fn valid_history_ids_are_the_old_and_the_new_shape() {
            assert!(is_valid_history_id("q-1"));
            assert!(is_valid_history_id("q-1736200000000"));
            assert!(is_valid_history_id(
                "q-1736200000000-0123456789abcdef0123456789abcdef"
            ));
        }

        #[test]
        fn ids_outside_the_two_shapes_are_rejected() {
            for id in [
                "",
                "q-",
                "q-abc",
                "Q-1",
                "q-1;DROP TABLE t",
                "../q-1",
                "q-1\r\nSet-Cookie: x=y",
                // The new shape, but not exactly: no epoch, an epoch that is
                // not digits, a suffix that is not 32 characters, an
                // uppercase hex digit, a character that is not hex at all,
                // and an empty part.
                "q--0123456789abcdef0123456789abcdef",
                "q-abc-0123456789abcdef0123456789abcdef",
                "q-1736200000000-0123456789abcdef0123456789abcde",
                "q-1736200000000-0123456789abcdef0123456789abcdef0",
                "q-1736200000000-0123456789ABCDEF0123456789ABCDEF",
                "q-1736200000000-0123456789abcdef0123456789abcdeg",
                "q-1736200000000-",
                "q-1736200000000--0123456789abcdef0123456789abcdef",
            ] {
                assert!(!is_valid_history_id(id), "expected rejected: {id:?}");
            }
        }
    }

    // ── GET /api/query/scheduling (WS2 §13, WS2 plan review W10, round 2, item 1) ──

    mod scheduling_pure {
        use super::*;

        #[test]
        fn scheduling_capability_reports_unsupported_with_a_reason() {
            let body = scheduling_capability_body();
            assert_eq!(body["supported"], serde_json::json!(false));
            assert!(body["reason"].as_str().unwrap().contains("WS7"));
        }
    }

    // ── WS7 item C2: `run` rewrites/refuses through `sql_rewrite::enforce`
    //    before EITHER engine ────────────────────────────────────────────

    mod enforcement {
        //! Real Postgres (`#[sqlx::test]`, a real authored `policy` row)
        //! plus a wiremock `ClickHouse` — the same two harnesses
        //! `tools::data::run_sql_delegation` uses for the equivalent
        //! proof one layer up (the copilot's `run_sql` tool). `ChClient`
        //! has no `last_query()`-style test double, so every assertion
        //! here reads the mock server's own recorded requests, matching
        //! this file's existing `trino_engine::assert_refused_before_reaching_trino`.

        #![allow(clippy::unwrap_used, clippy::expect_used)]

        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        use lakehouse_store::governance::CreatePolicyInput;
        use uuid::Uuid;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        fn database_url_for(pool: &PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        /// The state these tests run against: the seeded deployment's
        /// database, the mock engine, and the shared catalog's own
        /// tenant — the query path is scoped to the tenant, and the
        /// catalog's gate admits only that tenant's members.
        fn state_for(pool: &PgPool, ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT_ID.to_owned());
            AppState::new(Config::from_map(&env).expect("valid config from a map"))
        }

        /// The shared catalog belongs to one tenant, and the query path
        /// is scoped to the tenant: this fixture principal is the
        /// catalog's own member, so the shared tables stay reachable.
        const CATALOG_TENANT_ID: &str = "11111111-1111-4111-8111-000000000001";

        fn analyst_principal() -> Extension<Principal> {
            Extension(Principal {
                id: PrincipalId::User(Uuid::nil()),
                tenant_ids: vec![uuid::uuid!("11111111-1111-4111-8111-000000000001")],
                display_name: "alice".to_owned(),
                permissions: PermissionSet::parse("query:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: vec!["Analyst".to_owned()],
            })
        }

        async fn mount_governed_table_responses(server: &MockServer) {
            Mock::given(method("POST"))
                .and(body_string_contains("system.columns"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "name", "type": "String"},
                        {"name": "default_kind", "type": "String"},
                        {"name": "default_expression", "type": "String"},
                    ],
                    "data": [
                        {"name": "id", "default_kind": "", "default_expression": ""},
                        {"name": "email", "default_kind": "", "default_expression": ""},
                    ],
                    "rows": 2,
                })))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("system.tables"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "database", "type": "String"},
                        {"name": "name", "type": "String"},
                        {"name": "engine", "type": "String"},
                        {"name": "create_table_query", "type": "String"},
                    ],
                    "data": [
                        {"database": "serving", "name": "mart_x", "engine": "MergeTree", "create_table_query": ""},
                    ],
                    "rows": 1,
                })))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("replaceRegexpOne"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "id", "type": "UInt64"},
                        {"name": "email", "type": "String"},
                    ],
                    "data": [{"id": "1", "email": "***"}],
                    "rows": 1,
                })))
                .mount(server)
                .await;
        }

        /// Failing-test-first for WS7 item C2: before this task wired
        /// `sql_rewrite::enforce` into `run`, this assertion failed
        /// because `run` sent the LITERAL, unmasked `SELECT * FROM
        /// serving.mart_x` straight to `ClickHouse` — no request in
        /// `server.received_requests()` ever contained
        /// `"replaceRegexpOne"` at all. Quoted failure text (captured
        /// before Step 2's implementation): `assertion failed:
        /// requests.iter().any(|r| ...contains("replaceRegexpOne"))`.
        #[sqlx::test(migrations = "../../migrations")]
        async fn run_rewrites_sql_for_a_governed_table_before_executing(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            lakehouse_store::governance::create_policy(
                &pool,
                &CreatePolicyInput {
                    name: "query-run-masking-test".to_owned(),
                    kind: "Row filter".to_owned(),
                    subjects: "Analyst".to_owned(),
                    resources: "serving.mart_x".to_owned(),
                    effect: "Permit with obligation".to_owned(),
                    conditions: Some(
                        r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#
                            .to_owned(),
                    ),
                    activate: true,
                    owner: None,
                },
            )
            .await
            .expect("seeding the governing policy must succeed");

            let server = MockServer::start().await;
            mount_governed_table_responses(&server).await;

            let state = state_for(&pool, &server.uri());

            let body = Bytes::from(json!({ "sql": "SELECT * FROM serving.mart_x" }).to_string());
            let result = run(
                State(state),
                Some(analyst_principal()),
                HeaderMap::new(),
                body,
            )
            .await;
            assert!(
                result.is_ok(),
                "expected a successful, masked run: {result:?}"
            );

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests
                    .iter()
                    .any(|r| String::from_utf8_lossy(&r.body).contains("replaceRegexpOne")),
                "expected the query ClickHouse actually received to be the rewritten/masked \
                 form, never the original literal SQL"
            );
            Ok(())
        }

        /// Failing-test-first for WS7 item C2's Hard Requirement 2 (never
        /// silently pass an unrewritten/unrefused query through): before
        /// this task, `run` had no classification step at all, so a
        /// table-function read reached `ClickHouse` unrefused —
        /// `server.received_requests()` would have been non-empty.
        /// Quoted failure text (captured before Step 2's implementation):
        /// `assertion failed: requests.is_empty()`, `left: false, right:
        /// true` (a request WAS recorded, the table function reached
        /// `ClickHouse`).
        /// Step 1 of plan 15: the check refuses a write before `ClickHouse`
        /// is called. No mock is mounted, so a request reaching `ClickHouse`
        /// would also fail, but `requests.is_empty()` is the stronger proof.
        #[sqlx::test(migrations = "../../migrations")]
        async fn check_refuses_a_write_statement_before_touching_clickhouse(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            let state = state_for(&pool, &server.uri());

            let result = check_statement_would_run(
                &state,
                &analyst_principal().0,
                "DELETE FROM serving.mart_x",
                "clickhouse",
            )
            .await;
            assert!(result.is_err(), "a write must be refused: {result:?}");

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests.is_empty(),
                "a write must never reach ClickHouse at all: {requests:?}"
            );
            Ok(())
        }

        /// Step 1 of plan 15: a statement the policy refuses (a table
        /// function) is refused by the check before `ClickHouse` is called.
        #[sqlx::test(migrations = "../../migrations")]
        async fn check_refuses_a_policy_refused_statement_before_touching_clickhouse(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            let state = state_for(&pool, &server.uri());

            let result = check_statement_would_run(
                &state,
                &analyst_principal().0,
                "SELECT * FROM url('h','CSV')",
                "clickhouse",
            )
            .await;
            assert!(
                result.is_err(),
                "a table function must be refused: {result:?}"
            );

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests.is_empty(),
                "a refused statement must never reach ClickHouse at all: {requests:?}"
            );
            Ok(())
        }

        /// Step 1 of plan 15: the check sends `EXPLAIN PLAN` of the
        /// rewritten statement and nothing else, so the body starts with
        /// `EXPLAIN PLAN ` and holds the rewritten form, never the caller's
        /// literal `SELECT`, and its trailing `;` is stripped.
        #[sqlx::test(migrations = "../../migrations")]
        async fn check_sends_explain_plan_of_the_rewritten_statement(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            lakehouse_store::governance::create_policy(
                &pool,
                &CreatePolicyInput {
                    name: "check-explain-plan-test".to_owned(),
                    kind: "Row filter".to_owned(),
                    subjects: "Analyst".to_owned(),
                    resources: "serving.mart_x".to_owned(),
                    effect: "Permit with obligation".to_owned(),
                    conditions: Some(
                        r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#
                            .to_owned(),
                    ),
                    activate: true,
                    owner: None,
                },
            )
            .await
            .expect("seeding the governing policy must succeed");

            let server = MockServer::start().await;
            mount_governed_table_responses(&server).await;
            let state = state_for(&pool, &server.uri());

            let result = check_statement_would_run(
                &state,
                &analyst_principal().0,
                "SELECT * FROM serving.mart_x;",
                "clickhouse",
            )
            .await;
            assert!(
                result.is_ok(),
                "a governed read must pass the check: {result:?}"
            );

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            let explain: Vec<String> = requests
                .iter()
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .filter(|b| b.starts_with("EXPLAIN PLAN "))
                .collect();
            assert_eq!(
                explain.len(),
                1,
                "the check must send exactly one EXPLAIN PLAN: {explain:?}"
            );
            assert!(
                explain[0].contains("replaceRegexpOne"),
                "the EXPLAIN PLAN must hold the rewritten, masked statement: {}",
                explain[0]
            );
            assert!(
                !explain[0].contains(';'),
                "the trailing semicolon must be stripped before EXPLAIN PLAN: {}",
                explain[0]
            );
            Ok(())
        }

        /// Step 1 of plan 15: the masking proof on the check's path. The
        /// `EXPLAIN PLAN` body must carry the masking the caller would get
        /// from `run`. Mutation evidence: sending the caller's statement
        /// instead of the rewritten one makes this assertion fail.
        #[sqlx::test(migrations = "../../migrations")]
        async fn check_masks_a_governed_column_in_the_explain_plan_body(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            lakehouse_store::governance::create_policy(
                &pool,
                &CreatePolicyInput {
                    name: "check-masking-test".to_owned(),
                    kind: "Row filter".to_owned(),
                    subjects: "Analyst".to_owned(),
                    resources: "serving.mart_x".to_owned(),
                    effect: "Permit with obligation".to_owned(),
                    conditions: Some(
                        r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#
                            .to_owned(),
                    ),
                    activate: true,
                    owner: None,
                },
            )
            .await
            .expect("seeding the governing policy must succeed");

            let server = MockServer::start().await;
            mount_governed_table_responses(&server).await;
            let state = state_for(&pool, &server.uri());

            check_statement_would_run(
                &state,
                &analyst_principal().0,
                "SELECT * FROM serving.mart_x",
                "clickhouse",
            )
            .await
            .expect("a governed read must pass the check");

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests.iter().any(|r| {
                    let body = String::from_utf8_lossy(&r.body);
                    body.starts_with("EXPLAIN PLAN ") && body.contains("replaceRegexpOne")
                }),
                "the EXPLAIN PLAN ClickHouse receives must be the masked form: {requests:?}"
            );
            Ok(())
        }

        /// Step 1 of plan 15: the Trino arm is not built yet (plan 16), so
        /// it answers "not supported yet" and sends nothing to `ClickHouse`.
        #[sqlx::test(migrations = "../../migrations")]
        async fn check_answers_not_supported_yet_for_trino_without_clickhouse(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            let state = state_for(&pool, &server.uri());

            let result =
                check_statement_would_run(&state, &analyst_principal().0, "SELECT 1", "trino")
                    .await;
            let message = result.expect_err("the Trino arm must not pass the check yet");
            assert!(
                message.contains("not supported yet"),
                "the Trino arm must say it is not supported yet: {message}"
            );

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(requests.is_empty(), "no ClickHouse request: {requests:?}");
            Ok(())
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn run_refuses_a_table_function_before_touching_clickhouse_at_all(
            pool: PgPool,
        ) -> sqlx::Result<()> {
            let server = MockServer::start().await;
            // No mocks mounted at all — any request reaching `ClickHouse`
            // fails with a connection/404 error, which would itself prove
            // the refusal came too late.
            let state = state_for(&pool, &server.uri());

            let body = Bytes::from(json!({ "sql": "SELECT * FROM url('h','CSV')" }).to_string());
            let result = run(
                State(state),
                Some(analyst_principal()),
                HeaderMap::new(),
                body,
            )
            .await;
            let err = result.expect_err("a table function must be refused");
            assert_eq!(err.into_response().status(), 422);

            let requests = server
                .received_requests()
                .await
                .expect("mock server records requests");
            assert!(
                requests.is_empty(),
                "a table function must never reach ClickHouse at all: {requests:?}"
            );
            Ok(())
        }
    }
}

/// `SEC-20`: the tenant gate's rule, on its own. A separate module from
/// `tests` so the two do not share a hunk.
#[cfg(test)]
mod tenant_scope_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_store::uploads::TableClaim;
    use uuid::Uuid;

    use super::{
        TenantReach, first_unreachable, raw_table_name, reach_from, referenced_table_names,
        table_is_reachable, table_not_available,
    };

    const TENANT_A: Uuid = Uuid::from_u128(1);
    const TENANT_B: Uuid = Uuid::from_u128(2);

    /// The catalog's own tenant: it may read the shared tables.
    fn catalog_tenant() -> TenantReach {
        TenantReach::Tenant {
            tenant_id: TENANT_A,
            owns_catalog: true,
        }
    }

    /// Another tenant: it may read its own uploads, and nothing else.
    fn other_tenant() -> TenantReach {
        TenantReach::Tenant {
            tenant_id: TENANT_B,
            owns_catalog: false,
        }
    }

    #[test]
    fn an_upload_claimed_by_another_tenant_is_refused() {
        assert!(!table_is_reachable(other_tenant(), TableClaim::Theirs));
        assert!(!table_is_reachable(catalog_tenant(), TableClaim::Theirs));
    }

    #[test]
    fn an_upload_claimed_by_the_callers_own_tenant_is_allowed() {
        assert!(table_is_reachable(other_tenant(), TableClaim::Ours));
        assert!(table_is_reachable(catalog_tenant(), TableClaim::Ours));
    }

    #[test]
    fn a_shared_table_follows_the_catalogs_own_gate() {
        assert!(!table_is_reachable(other_tenant(), TableClaim::Unclaimed));
        assert!(table_is_reachable(catalog_tenant(), TableClaim::Unclaimed));
    }

    #[test]
    fn a_principal_holding_every_permission_passes() {
        assert!(table_is_reachable(TenantReach::All, TableClaim::Theirs));
        assert!(table_is_reachable(TenantReach::All, TableClaim::Unclaimed));
    }

    #[test]
    fn a_deployment_with_one_tenant_changes_nothing() {
        assert!(table_is_reachable(TenantReach::All, TableClaim::Ours));
        assert!(table_is_reachable(TenantReach::All, TableClaim::Theirs));
        assert!(table_is_reachable(TenantReach::All, TableClaim::Unclaimed));
    }

    #[test]
    fn a_principal_with_no_tenant_reaches_nothing() {
        assert!(!table_is_reachable(TenantReach::NoTenant, TableClaim::Ours));
        assert!(!table_is_reachable(
            TenantReach::NoTenant,
            TableClaim::Unclaimed
        ));
    }

    #[test]
    fn a_raw_upload_name_is_the_bronze_namespace_and_the_raw_name() {
        assert_eq!(raw_table_name("bronze.orders"), Some("orders"));
        assert_eq!(raw_table_name("bronze"), None);
        assert_eq!(raw_table_name("silver.orders"), None);
        assert_eq!(raw_table_name("serving.mart_x"), None);
        assert_eq!(raw_table_name("orders"), None);
    }

    #[test]
    fn every_sql_form_of_a_bronze_table_names_the_same_upload() {
        for sql in [
            "SELECT * FROM bronze.orders",
            "SELECT * FROM iceberg.bronze.orders",
            "SELECT * FROM icecat_x.`bronze.orders`",
        ] {
            assert_eq!(
                referenced_table_names(sql, false),
                vec!["bronze.orders".to_owned()],
                "{sql}"
            );
        }
    }

    #[test]
    fn a_statement_reading_two_tables_refuses_on_the_second() {
        let tables = referenced_table_names(
            "SELECT * FROM bronze.invoices JOIN silver.orders ON true",
            false,
        );
        assert_eq!(
            tables,
            vec!["bronze.invoices".to_owned(), "silver.orders".to_owned()]
        );
        // The caller's own upload first, then a shared table it may not
        // read: the refusal names the second.
        let claims = [TableClaim::Ours, TableClaim::Unclaimed];
        assert_eq!(first_unreachable(other_tenant(), &claims), Some(1));
        assert_eq!(first_unreachable(catalog_tenant(), &claims), None);
    }

    #[test]
    fn a_cte_named_like_a_foreign_table_is_not_a_table() {
        let tables = referenced_table_names(
            "WITH foreign_orders AS (SELECT 1) SELECT * FROM foreign_orders",
            false,
        );
        assert!(tables.is_empty(), "{tables:?}");
    }

    #[test]
    fn the_refusal_is_a_404_with_the_fixed_sentence() {
        let err = table_not_available("bronze.orders");
        assert_eq!(err.status(), 404);
        assert_eq!(
            err.to_string(),
            "Table not found or not available to your organisation: bronze.orders"
        );
    }

    #[test]
    fn a_deployment_with_one_tenant_reaches_everything() {
        assert_eq!(
            reach_from(false, Some(TENANT_A), Some(1), None),
            TenantReach::All
        );
        assert_eq!(reach_from(false, None, Some(1), None), TenantReach::All);
    }

    #[test]
    fn the_catalogs_own_tenant_owns_the_shared_tables() {
        assert_eq!(
            reach_from(false, Some(TENANT_A), Some(2), Some(TENANT_A)),
            TenantReach::Tenant {
                tenant_id: TENANT_A,
                owns_catalog: true,
            }
        );
    }

    #[test]
    fn a_gate_that_admits_a_caller_that_does_not_own_the_catalog_is_narrowed() {
        // The gate read a smaller count than this request did: the
        // shared tables are the catalog tenant's, not the caller's.
        assert_eq!(
            reach_from(false, Some(TENANT_B), Some(2), Some(TENANT_A)),
            TenantReach::Tenant {
                tenant_id: TENANT_B,
                owns_catalog: false,
            }
        );
        // And a caller that belongs to no tenant reaches nothing at all.
        assert_eq!(
            reach_from(false, None, Some(2), Some(TENANT_A)),
            TenantReach::NoTenant
        );
    }

    #[test]
    fn a_closed_gate_keeps_the_caller_to_its_own_tenant() {
        assert_eq!(
            reach_from(true, Some(TENANT_B), Some(2), Some(TENANT_A)),
            TenantReach::Tenant {
                tenant_id: TENANT_B,
                owns_catalog: false,
            }
        );
        assert_eq!(
            reach_from(true, None, Some(2), Some(TENANT_A)),
            TenantReach::NoTenant
        );
    }

    #[test]
    fn an_unknown_tenant_count_is_never_the_one_tenant_case() {
        assert_eq!(
            reach_from(false, Some(TENANT_A), None, None),
            TenantReach::Tenant {
                tenant_id: TENANT_A,
                owns_catalog: false,
            }
        );
    }
}

/// `SEC-50`: the shared tenant gate's actors, on their own — which reach
/// each actor gets, and what the gate refuses.
#[cfg(test)]
mod sec50_gate_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::http::HeaderMap;
    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use lakehouse_store::PgPool;
    use lakehouse_store::uploads::claim_table;
    use uuid::Uuid;

    use super::{
        GateActor, JOB_NEEDS_CATALOG_TENANT, TenantReach, catalog_tenant_reach, gate_statement,
        reach_for_actor,
    };
    use crate::config::Config;
    use crate::state::AppState;

    /// The tenant that owns the shared catalog in these tests.
    const CATALOG_TENANT: Uuid = uuid::uuid!("11111111-1111-4111-8111-000000000001");
    /// Another tenant of the same deployment.
    const OTHER_TENANT: Uuid = uuid::uuid!("11111111-1111-4111-8111-000000000002");
    /// The upload table the claims below are about.
    const CLAIMED_TABLE: &str = "group_orders";
    /// The statement that reads it.
    const STATEMENT: &str = "SELECT * FROM bronze.group_orders";
    /// The refusal of that statement, byte for byte.
    const REFUSAL: &str =
        "Table not found or not available to your organisation: bronze.group_orders";

    /// A principal that belongs to exactly one tenant.
    fn principal_in(tenant_id: Uuid) -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: vec![tenant_id],
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("query:read"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// `SEC-50` R1: a principal holding the unrestricted (`"*:*"`) grant.
    /// It belongs to no tenant at all, exactly like the bootstrap admin
    /// the G7 harness creates its pipelines as.
    fn unrestricted_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "Bootstrap Admin".to_owned(),
            permissions: PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A state with no pool at all: the deployment's own answers are
    /// missing, so the gate must fail closed rather than guess.
    fn state_without_a_database() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).expect("a config with no pool"))
    }

    /// The pool's own URL, for a state that reads it.
    fn database_url_for(pool: &PgPool) -> String {
        let options = pool.connect_options();
        format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options
                .get_database()
                .expect("#[sqlx::test] always targets a named database"),
        )
    }

    #[test]
    fn a_job_with_no_user_reaches_everything_in_a_one_tenant_deployment() {
        assert_eq!(
            catalog_tenant_reach(Some(1), None).expect("a deployment with one tenant"),
            TenantReach::All
        );
    }

    #[test]
    fn a_job_with_no_user_acts_as_the_tenant_that_owns_the_shared_catalog() {
        assert_eq!(
            catalog_tenant_reach(Some(4), Some(CATALOG_TENANT)).expect("the catalog's own tenant"),
            TenantReach::Tenant {
                tenant_id: CATALOG_TENANT,
                owns_catalog: true,
            }
        );
    }

    #[test]
    fn a_job_with_no_user_is_refused_when_no_tenant_owns_the_shared_catalog() {
        let err = catalog_tenant_reach(Some(4), None)
            .expect_err("several tenants and no tenant that owns the shared catalog");
        assert_eq!(err.status(), 422);
        assert_eq!(err.to_string(), JOB_NEEDS_CATALOG_TENANT);
        // The sentence names the setting and what to do with it, and
        // nothing about the deployment's data.
        assert_eq!(
            JOB_NEEDS_CATALOG_TENANT,
            "This job reads the shared catalog, and no tenant owns it here. Set CATALOG_TENANT_ID to the tenant that owns this deployment's shared catalog."
        );
    }

    /// `SEC-50` R5 (amended, PR #281 review round 2 NIT): a deployment
    /// that cannot COUNT its tenants refuses the job, even with the setting
    /// named. "Cannot verify it is safe" is the same answer as "verified
    /// unsafe" — `SEC-20`'s own rule, the one the shared catalog's own gate
    /// already follows with no pool.
    ///
    /// The sentence is `SEC-20`'s own for that state ("could not verify
    /// tenant count", a 503), NOT D6's: D6 tells the operator to set a
    /// setting that may already be set, and the thing that is missing here
    /// is the deployment's own answer, not the setting.
    #[test]
    fn a_job_with_no_user_is_refused_in_a_deployment_that_cannot_count_its_tenants() {
        let err = catalog_tenant_reach(None, Some(CATALOG_TENANT))
            .expect_err("a deployment that cannot say how many tenants it holds");
        assert_eq!(err.status(), 503);
        assert_eq!(
            err.to_string(),
            crate::routes::catalog::COULD_NOT_VERIFY_TENANT_COUNT
        );
        assert_eq!(
            err.to_string(),
            "could not verify tenant count",
            "the sentence names the count, not a setting to set"
        );
    }

    #[tokio::test]
    async fn a_site_that_acts_as_no_tenant_reaches_nothing() {
        let state = state_without_a_database();
        let principal = principal_in(CATALOG_TENANT);
        let reach = reach_for_actor(
            &state,
            GateActor::AsTenant {
                principal: &principal,
                tenant_id: None,
            },
        )
        .await
        .expect("the reach of a site with no tenant at all");
        assert_eq!(reach, TenantReach::NoTenant);
    }

    /// `SEC-50` R1 (review round 1, BLOCKER 1): a principal holding the
    /// unrestricted grant reaches everything, even through a site that
    /// acts as no tenant at all. `SEC-20`'s own rule is that such a
    /// principal passes, and it reads every table in Query Studio
    /// already.
    #[tokio::test]
    async fn an_unrestricted_principal_reaches_everything_even_as_a_site_with_no_tenant() {
        let state = state_without_a_database();
        let principal = unrestricted_principal();
        let reach = reach_for_actor(
            &state,
            GateActor::AsTenant {
                principal: &principal,
                tenant_id: None,
            },
        )
        .await
        .expect("the reach of an unrestricted principal");
        assert_eq!(reach, TenantReach::All);
    }

    /// `SEC-50` R1: the same rule for a site that acts as a NAMED tenant
    /// the principal does not belong to — the unrestricted grant is
    /// checked before the tenant, so the principal is not refused for a
    /// tenant it never had.
    #[tokio::test]
    async fn an_unrestricted_principal_reaches_everything_even_as_a_tenant_it_is_not_in() {
        let state = state_without_a_database();
        let principal = unrestricted_principal();
        let reach = reach_for_actor(
            &state,
            GateActor::AsTenant {
                principal: &principal,
                tenant_id: Some(OTHER_TENANT),
            },
        )
        .await
        .expect("the reach of an unrestricted principal");
        assert_eq!(reach, TenantReach::All);
    }

    #[tokio::test]
    async fn a_site_that_acts_as_a_tenant_it_may_not_act_as_is_refused() {
        let state = state_without_a_database();
        let principal = principal_in(CATALOG_TENANT);
        let err = reach_for_actor(
            &state,
            GateActor::AsTenant {
                principal: &principal,
                tenant_id: Some(OTHER_TENANT),
            },
        )
        .await
        .expect_err("a principal that is not a member of that tenant");
        // The same 404 an `X-Tenant` header naming a foreign tenant
        // gets — never a 403, which would confirm the tenant exists.
        assert_eq!(err.status(), 404);
        assert_eq!(err.to_string(), "tenant not found");
    }

    /// `SEC-50` R5 (amended, PR #281 review round 2 NIT): a deployment
    /// with no database at all cannot say who owns the shared catalog, so
    /// the job-with-no-user actor is refused with `SEC-20`'s own sentence
    /// for that state (a 503) — not D6's, which names a setting to set.
    #[tokio::test]
    async fn a_job_with_no_user_in_a_deployment_that_cannot_say_who_owns_the_catalog_is_refused() {
        let state = state_without_a_database();
        let err = reach_for_actor(&state, GateActor::CatalogTenant)
            .await
            .expect_err("a deployment that cannot say who owns the shared catalog");
        assert_eq!(err.status(), 503);
        assert_eq!(
            err.to_string(),
            crate::routes::catalog::COULD_NOT_VERIFY_TENANT_COUNT
        );
    }

    /// The gate itself, over a real deployment: the upload table another
    /// tenant claimed is out of reach for every actor that does not own
    /// it, and in reach for the tenant that does.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_gate_refuses_another_tenants_upload_table_for_every_actor(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url_for(&pool));
        env.insert("CH_URL".to_owned(), "http://127.0.0.1:1".to_owned());
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_string());
        let state = AppState::new(Config::from_map(&env).expect("valid config from a map"));

        let claimed = claim_table(&pool, OTHER_TENANT, CLAIMED_TABLE, "upload-other")
            .await
            .expect("claim the upload table for the other tenant");
        assert!(claimed, "the other tenant must hold the claim");

        let catalog_principal = principal_in(CATALOG_TENANT);
        let other_principal = principal_in(OTHER_TENANT);

        // The caller in the catalog's own tenant may not read the other
        // tenant's upload.
        let refused = gate_statement(
            &state,
            STATEMENT,
            &sqlparser::dialect::ClickHouseDialect {},
            GateActor::Caller {
                principal: &catalog_principal,
                headers: &HeaderMap::new(),
            },
        )
        .await
        .expect_err("another tenant's upload table");
        assert_eq!(refused.to_string(), REFUSAL);

        // A job with no user acts as the catalog's tenant, so the other
        // tenant's upload is out of ITS reach too.
        let refused = gate_statement(
            &state,
            STATEMENT,
            &sqlparser::dialect::ClickHouseDialect {},
            GateActor::CatalogTenant,
        )
        .await
        .expect_err("another tenant's upload table");
        assert_eq!(refused.to_string(), REFUSAL);

        // The tenant that holds the table reads it — as a caller acting
        // as itself, and as a site acting as that tenant.
        let allowed = gate_statement(
            &state,
            STATEMENT,
            &sqlparser::dialect::ClickHouseDialect {},
            GateActor::AsTenant {
                principal: &other_principal,
                tenant_id: Some(OTHER_TENANT),
            },
        )
        .await;
        assert!(allowed.is_ok(), "the owning tenant reads its own upload");
        Ok(())
    }
}

/// `QS-4` step 4: the history query string's parser, on its own. The
/// store's tests own the SQL semantics; these own the 400s.
#[cfg(test)]
mod history_query_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{HistoryQueryRaw, parse_history_query};

    fn raw(
        status: Option<&str>,
        engine: Option<&str>,
        min_duration_ms: Option<&str>,
        table: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        before: Option<&str>,
    ) -> HistoryQueryRaw {
        HistoryQueryRaw {
            status: status.map(str::to_owned),
            engine: engine.map(str::to_owned),
            min_duration_ms: min_duration_ms.map(str::to_owned),
            table: table.map(str::to_owned),
            from: from.map(str::to_owned),
            to: to.map(str::to_owned),
            before: before.map(str::to_owned),
        }
    }

    /// A well-formed query string lands in the filter with every field it
    /// named, and one that named nothing filters nothing.
    #[test]
    fn a_good_query_parses_into_its_filter() {
        let query = raw(
            Some("failed"),
            Some("trino"),
            Some("1500"),
            Some("orders"),
            Some("2026-10-11T00:00:00Z"),
            Some("2026-10-20T12:00:00.500Z"),
            Some("2026-10-20T11:59:59.999Z,q-1729"),
        );
        let filter = parse_history_query(&query).unwrap();
        assert_eq!(filter.status, Some("failed"));
        assert_eq!(filter.engine, Some("trino"));
        assert_eq!(filter.min_duration_ms, Some(1500));
        assert_eq!(filter.table, Some("orders"));
        assert_eq!(
            filter.before_id,
            Some("q-1729"),
            "the cursor's id half is everything after the first comma"
        );
        assert!(filter.from.is_some() && filter.to.is_some() && filter.before_at.is_some());

        let empty_query = raw(None, None, None, None, None, None, None);
        let empty = parse_history_query(&empty_query).unwrap();
        assert_eq!(empty, super::HistoryFilter::default());
    }

    /// An empty value is no value: a client that builds links from empty
    /// inputs filters nothing instead of getting a 400.
    #[test]
    fn empty_values_filter_nothing() {
        let query = raw(
            Some(""),
            Some(""),
            Some(""),
            Some(""),
            Some(""),
            Some(""),
            Some(""),
        );
        let filter = parse_history_query(&query).unwrap();
        assert_eq!(filter, super::HistoryFilter::default());
    }

    /// A value outside the known sets is a 400 whose message names the
    /// field — fixed text, never a serde rendering.
    #[test]
    fn an_unknown_status_or_engine_is_a_400_naming_the_field() {
        let bad_status = raw(Some("yesterday"), None, None, None, None, None, None);
        let err = parse_history_query(&bad_status).unwrap_err();
        assert_eq!(
            err.to_string(),
            "status must be one of completed, failed, cancelled, blocked"
        );
        let bad_engine = raw(None, Some("presto"), None, None, None, None, None);
        let err = parse_history_query(&bad_engine).unwrap_err();
        assert_eq!(err.to_string(), "engine must be one of clickhouse, trino");
    }

    /// A value that does not parse is a 400 whose message names the field:
    /// the duration's number, the window's times, and the cursor's two
    /// halves.
    #[test]
    fn a_bad_duration_time_or_cursor_is_a_400_naming_the_field() {
        let cases: [(HistoryQueryRaw, &str); 6] = [
            (
                raw(None, None, Some("soon"), None, None, None, None),
                "minDurationMs",
            ),
            (
                raw(None, None, Some("-1"), None, None, None, None),
                "minDurationMs",
            ),
            (
                raw(None, None, None, None, Some("not-a-time"), None, None),
                "from",
            ),
            (
                raw(
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("2026-13-45T00:00:00Z"),
                    None,
                ),
                "to",
            ),
            (
                raw(
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("2026-10-20T00:00:00Z"),
                ),
                "before",
            ),
            (
                raw(
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some("2026-10-20T00:00:00Z,"),
                ),
                "before",
            ),
        ];
        for (query, field) in &cases {
            let err = parse_history_query(query).unwrap_err();
            let message = err.to_string();
            assert!(message.starts_with(field), "{message} must name {field:?}");
            assert_eq!(err.status(), 400, "a bad value is a 400: {message}");
        }
    }
}
