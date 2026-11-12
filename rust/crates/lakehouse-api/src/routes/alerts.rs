//! `GET/POST/PUT/DELETE /api/alerts`, `GET/POST /api/alerts/run` and
//! `POST /api/alerts/test` — threshold alerts & scheduled digests.
//!
//! Ports `src/app/api/alerts/route.ts` and
//! `src/app/api/alerts/run/route.ts`.
//!
//! Two doors run rules, and they answer different callers (`BI-40` D1). The
//! scheduler's is `GET/POST /api/alerts/run`: [`run`] checks
//! [`check_run_token`] first, so only the cron's shared token or a service
//! identity gets through (D4), and it drives the whole tick — rules, then the
//! quality, schema, semantic, lineage and late-pipeline passes. A person's is
//! `POST /api/alerts/test?id=`: [`test_one`] asks [`authorise_rule_test`] who
//! may manage that one rule and runs it through [`run_one`], the one wiring
//! both doors (and the assistant's `run_alert_rule`) share.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Extension, Query, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_alerts::{
    AlertKind, AlertRule, AlertRuleInput, AlertState, FreshnessSource, GateRefusal, LateSource,
    SilenceSource, SqlGate, StateSource,
};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_iceberg::IcebergClient;
use lakehouse_notify::EmailSender;
use lakehouse_store::PgPool;
use lakehouse_store::overview::{self, FiredRule};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

/// `GET /api/alerts` — list the alert & digest rules the caller may see.
///
/// The body is `{"rules": [...]}`, as the `TypeScript` handler's. Rules of
/// the pre-`SRC-7` kinds are returned exactly as before; rules of the six
/// `SRC-7` kinds are filtered by [`visible_rules`] (`SRC-7` review BLOCKER 1:
/// the list is one for the installation, and such a rule names a connector
/// and carries a webhook target). Whether run reports can arrive is a
/// separate route, [`status`], so this body stays what the parity corpus
/// recorded (`SRC-7` review SHOULD-FIX 3).
///
/// `DEC-16` (QA `LH-SRC-7-04`, `SRC-42`): a rule's `target` is returned only
/// to a caller who may manage the rule; everyone else gets the rule with
/// `"targetHidden": true` ([`rule_json_for`]).
///
/// The `TypeScript` handler's `catch` returns a 500 with `e.message`
/// (`alerts/route.ts`'s `GET`), unlike `POST`/`PUT` which return 400 for
/// the same kind of failure — the status code here depends on which route
/// caught the error, not on the error's own type, so it is chosen at each
/// call site rather than baked into a single `From` conversion.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] on a `ClickHouse` failure; for the
/// `SRC-7` kinds, whatever [`visible_rules`] returns.
pub async fn list(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let rules = lakehouse_alerts::list_rules(&state.clickhouse)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::Internal,
            )
        })?;
    let rules = listed_rules(&state, principal.as_ref().map(|Extension(p)| p), rules).await?;
    Ok(ApiJson(json!({ "rules": rules })))
}

/// What a chart rule's row says about the chart it watches (`BI-39` D8), read
/// live: the title is never stored, so a rename shows at once and a chart the
/// caller lost access to stops naming itself (the same reason the bell is built
/// live, `BI-19` part E deviation 1).
struct ChartTitle {
    title: String,
    board_id: String,
    board_title: String,
}

/// A rule as the list's JSON, with its `target` only for a caller who may
/// manage it (`DEC-16`, QA `LH-SRC-7-04`, `SRC-42`).
///
/// A webhook address can carry a token and an e-mail address is personal, so a
/// caller with `alert:read` and nothing more sees `channel` (Webhook or
/// E-mail) but not where the rule sends: the key is left out and
/// `"targetHidden": true` says so. This is the one place — `GET /api/alerts`
/// and the assistant's `list_alert_rules` both go through [`listed_rules`]
/// (D15) — and the test of who may manage a rule is
/// [`chart_alerts::may_manage`], the same one the write routes use.
///
/// # Errors
///
/// The serializer's error, which the caller turns into a fixed 500.
fn rule_json_for(
    principal: Option<&Principal>,
    rule: &AlertRule,
) -> Result<Value, serde_json::Error> {
    let mut value = serde_json::to_value(rule)?;
    if !crate::routes::chart_alerts::may_manage(principal, Some(rule))
        && let Some(map) = value.as_object_mut()
    {
        map.remove("target");
        map.insert("targetHidden".to_owned(), json!(true));
    }
    Ok(value)
}

/// The rules of `rules` the caller may see, each as the list's JSON
/// (`GET /api/alerts` and the assistant's `list_alert_rules`, one helper so the
/// two cannot drift — rule 4).
///
/// `BI-39` D8: a chart rule whose chart the caller can see **now** also carries
/// `chartTitle`, `dashboardId` and `dashboardTitle`; a chart rule whose chart
/// they cannot see carries none of the three, and the console keeps
/// "Dashboard chart". The caller's charts and boards are read once for the whole
/// list, never once per rule.
///
/// `DEC-16` (`SRC-42`): a rule's target is included only for a caller who may
/// manage that rule ([`rule_json_for`]).
///
/// # Errors
///
/// As [`visible_rules`]; a rule that cannot be serialized is a 500 with a fixed
/// sentence (it cannot happen for this type, and is never guessed around).
pub(crate) async fn listed_rules(
    state: &AppState,
    principal: Option<&Principal>,
    rules: Vec<AlertRule>,
) -> Result<Vec<Value>, ApiError> {
    let rules = visible_rules(state, principal, rules).await?;
    let titles = if rules.iter().any(|rule| rule.chart.is_some()) {
        chart_titles(state, principal).await
    } else {
        HashMap::new()
    };
    let mut out = Vec::with_capacity(rules.len());
    for rule in rules {
        let mut value = rule_json_for(principal, &rule).map_err(|err| {
            tracing::warn!(%err, rule_id = %rule.id, "an alert rule could not be serialized");
            ApiError::Internal("the alert rules could not be listed".to_owned())
        })?;
        let found = rule.chart.as_deref().and_then(|id| titles.get(id));
        if let Some(found) = found
            && let Some(map) = value.as_object_mut()
        {
            map.insert("chartTitle".to_owned(), json!(found.title));
            map.insert("dashboardId".to_owned(), json!(found.board_id));
            map.insert("dashboardTitle".to_owned(), json!(found.board_title));
        }
        out.push(value);
    }
    Ok(out)
}

/// Chart id → its title and its dashboard's id and name, for the charts
/// `principal` can see now (`BI-39` D8). One read of the charts and one of the
/// boards; the built-in board's row, which `list_boards` leaves out, is read
/// once more only when a chart sits on it.
///
/// A store that cannot answer, or a caller with no principal, leaves the map
/// empty: the list then names no chart and the console shows its own fallback
/// text. Never a made-up title (principle 2).
async fn chart_titles(
    state: &AppState,
    principal: Option<&Principal>,
) -> HashMap<String, ChartTitle> {
    let Some(principal) = principal else {
        return HashMap::new();
    };
    let Ok(viewer) = crate::routes::dashboard_access::viewer_for(state, principal).await else {
        return HashMap::new();
    };
    let ch = &state.clickhouse;
    let Ok(charts) = lakehouse_bi::store::list_stored_charts(ch, &viewer).await else {
        return HashMap::new();
    };
    let Ok(boards) = lakehouse_bi::store::list_boards(ch, &viewer).await else {
        return HashMap::new();
    };
    let mut names: HashMap<String, String> = boards
        .into_iter()
        .map(|board| (board.id, board.name))
        .collect();
    // The built-in board's layout row is kept out of `list_boards`, and a chart
    // on it (stored with no board, or as `default`) is still the caller's to
    // see: its name comes from `get_board`, which answers a shell board when no
    // row was ever saved.
    if charts
        .iter()
        .any(|chart| chart.board.is_empty() || chart.board == lakehouse_bi::store::DEFAULT_BOARD_ID)
        && let Ok(Some(builtin)) =
            lakehouse_bi::store::get_board(ch, lakehouse_bi::store::DEFAULT_BOARD_ID, &viewer).await
    {
        names.entry(builtin.id).or_insert(builtin.name);
    }
    charts
        .into_iter()
        .filter_map(|chart| {
            let board_id = if chart.board.is_empty() {
                lakehouse_bi::store::DEFAULT_BOARD_ID.to_owned()
            } else {
                chart.board.clone()
            };
            let board_title = names.get(&board_id)?.clone();
            Some((
                chart.spec.id.clone(),
                ChartTitle {
                    title: chart.spec.title,
                    board_id,
                    board_title,
                },
            ))
        })
        .collect()
}

/// Body of `GET /api/alerts/status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertsStatus {
    /// True when this API has `PIPELINE_RUN_TOKEN` set (non-empty), the
    /// condition under which the orchestrator's run sensors can post failed
    /// and finished runs.
    run_events_configured: bool,
}

/// `GET /api/alerts/status` — whether connector and upload alerts can fire
/// at all (`SRC-7` D9): unset, no run report is accepted, and the console
/// says so instead of showing a silent "no alerts". It cannot see the
/// orchestrator's own copy of the token.
///
/// # Errors
///
/// None; the signature is the handlers' shared shape.
#[allow(
    clippy::unused_async,
    reason = "axum handlers are async fns; the body reads config only"
)]
pub async fn status(State(state): State<AppState>) -> ApiResult<ApiJson<AlertsStatus>> {
    Ok(ApiJson(AlertsStatus {
        run_events_configured: state.config.pipeline_run_token.is_some(),
    }))
}

/// The rules of `rules` that `principal` may see (`SRC-7` review BLOCKER 1).
///
/// Rules of the pre-`SRC-7` kinds pass untouched. For the six new kinds an
/// unrestricted caller ([`crate::routes::catalog::is_unrestricted`]) sees
/// all; anyone else sees only the rules scoped to a connector of their
/// tenants. A `*` rule, an `upload_failure` rule and a rule whose connector
/// no longer exists (or has no tenant) are hidden from them, as the save
/// check refuses the same scopes. Membership is decided for all connectors
/// in one query, not one per rule.
///
/// # Errors
///
/// 401 with no principal when a rule of the new kinds exists; 503 if no
/// pool is configured; 500 on a database failure.
pub(crate) async fn visible_rules(
    state: &AppState,
    principal: Option<&Principal>,
    rules: Vec<AlertRule>,
) -> Result<Vec<AlertRule>, ApiError> {
    // BI-19 part C: anybody signed in may now save a chart alert for
    // themselves, so the list shows each person their own (the alert permission
    // sees all); the rule names a target address and the chart's filters.
    let rules = crate::routes::chart_alerts::visible_to(state, rules, principal).await;
    if !rules
        .iter()
        .any(|rule| rule.kind.is_connector_scoped() || rule.kind.is_asset_scoped())
    {
        return Ok(rules);
    }
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    if crate::routes::catalog::is_unrestricted(principal) {
        return Ok(rules);
    }
    let mut ids: Vec<String> = rules
        .iter()
        .filter(|rule| rule.kind.is_connector_scoped())
        .filter_map(|rule| rule.connector.clone())
        .filter(|connector| connector != "*")
        .collect();
    ids.sort();
    ids.dedup();
    let own: std::collections::HashSet<String> = if ids.is_empty() {
        std::collections::HashSet::new()
    } else {
        let pool = state
            .pg
            .as_deref()
            .ok_or_else(|| ApiError::Unavailable("connector store unavailable".to_owned()))?;
        lakehouse_store::connectors::connector_ids_in_tenants(pool, &ids, &principal.tenant_ids)
            .await?
            .into_iter()
            .collect()
    };
    Ok(rules
        .into_iter()
        .filter(|rule| {
            // `DATA-14`: a quality incident rule is installation-wide, like a
            // `*` connector rule, so only an unrestricted caller (returned
            // above) sees it.
            !rule.kind.is_asset_scoped()
                && (!rule.kind.is_connector_scoped()
                    || rule
                        .connector
                        .as_deref()
                        .is_some_and(|connector| own.contains(connector)))
        })
        .collect())
}

/// Parse the raw request body as JSON into an [`AlertRuleInput`].
///
/// The `TypeScript` handler's `catch` around `await req.json()` swallows
/// *any* failure from that expression — a genuinely malformed body, but
/// also (irrelevantly here) a body read error — and reports it at 400 with
/// `e.message`. Bun's JSON parser produces a distinctive message (e.g.
/// `JSON Parse error: Unexpected identifier "not"`) that `serde_json`
/// cannot reproduce; see `rust/tests/parity/README.md`'s "Known
/// non-deterministic captures" section, which already documents this exact
/// pair of routes (`alerts-create-bad-body`,
/// `dashboard-boards-create-bad-body`) as un-portable runtime error text
/// and normalizes the `error` field for them in the parity harness. This
/// produces a sensible equivalent — a 400 naming the parse failure — rather
/// than contorting the parser to chase Bun's exact wording.
fn parse_body(body: &Bytes) -> Result<AlertRuleInput, ApiError> {
    serde_json::from_slice(body)
        .map_err(|err| ApiError::BadRequest(format!("JSON is invalid: {err}")))
}

/// The refusal for a rule scoped to every connector or to uploads, by
/// someone who does not see every tenant (`SRC-7` D7).
const ALL_SCOPE_DENIED: &str =
    "only an administrator who sees every tenant can save a rule for all connectors or for uploads";

/// The refusal for a quality incident rule by someone who does not see every
/// tenant (`DATA-14`).
const QUALITY_SCOPE_DENIED: &str =
    "only an administrator who sees every tenant can save a rule for quality incidents";

/// Who may save a rule of the `SRC-7` kinds (decision D7, finding F7). Rules
/// are one list for the installation, so a rule for all connectors, or for
/// uploads, would carry every tenant's connector and file names:
///
/// - scope `"*"` (and every `upload_failure`, which is always `"*"`) needs
///   [`crate::routes::catalog::is_unrestricted`];
/// - a connector id must pass
///   [`crate::routes::connectors::ensure_connector_in_tenants`], which
///   answers a connector outside the caller's tenants exactly as an unknown
///   one (404), so the answer is no oracle for which ids exist. An
///   unrestricted caller needs only the connector to exist.
///
/// Any other kind is not checked here (unchanged behaviour). A missing
/// connector is left to `save_rule`'s own validation (400). With no
/// principal the `SRC-7` kinds fail closed (401); the HTTP routes always
/// have one, and the copilot's alert tools pass theirs.
///
/// # Errors
///
/// 401, 403 or 404 as above; 503 if no pool is configured; 500 on a
/// database failure.
pub(crate) async fn authorise_rule_scope(
    state: &AppState,
    principal: Option<&Principal>,
    kind: Option<&str>,
    connector: Option<&str>,
) -> Result<(), ApiError> {
    // `DATA-14`: a quality incident rule's message names tables and rules
    // that may belong to any tenant, and its scope is a table, not a
    // connector the tenant check could test. Fail closed: only an
    // unrestricted caller saves, changes or deletes one.
    if kind.is_some_and(|k| AlertKind::parse_asset_kind(k).is_some()) {
        let Some(principal) = principal else {
            return Err(ApiError::unauthorized());
        };
        return if crate::routes::catalog::is_unrestricted(principal) {
            Ok(())
        } else {
            Err(ApiError::PermissionDenied(QUALITY_SCOPE_DENIED.to_owned()))
        };
    }
    let Some(kind) = kind.and_then(AlertKind::parse_connector_kind) else {
        return Ok(());
    };
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    let scope = if kind == AlertKind::UploadFailure {
        "*"
    } else {
        connector.map_or("", str::trim)
    };
    if scope.is_empty() {
        return Ok(());
    }
    if scope == "*" {
        return if crate::routes::catalog::is_unrestricted(principal) {
            Ok(())
        } else {
            Err(ApiError::PermissionDenied(ALL_SCOPE_DENIED.to_owned()))
        };
    }
    if crate::routes::catalog::is_unrestricted(principal) {
        // PR #101 CI (`an_administrator_may_save_a_rule_for_all_connectors_and_for_uploads`):
        // an administrator who sees every tenant is a member of none in
        // particular, so the tenant-membership test below refused a real
        // connector of another tenant. Existence is all that is left to
        // check, and an unknown id gets the same 404 as everywhere else.
        let pool = crate::routes::connectors::pool(state)?;
        return if lakehouse_store::connectors::get_connector(pool, scope)
            .await?
            .is_some()
        {
            Ok(())
        } else {
            Err(ApiError::NotFound(format!("Connector {scope} not found")))
        };
    }
    crate::routes::connectors::ensure_connector_in_tenants(state, Some(principal), scope).await
}

/// [`authorise_rule_scope`] for `input`, and for the rule `id` it replaces:
/// an update must not let a caller take over, or move, a rule they could not
/// have created (`SRC-7` D7). An unrestricted caller may do both, so the
/// stored rule is not even read for them.
///
/// # Errors
///
/// As [`authorise_rule_scope`]; 503 with a fixed message when the stored
/// rule cannot be read.
pub(crate) async fn authorise_rule_update(
    state: &AppState,
    principal: Option<&Principal>,
    headers: &HeaderMap,
    id: &str,
    input: &AlertRuleInput,
) -> Result<(), ApiError> {
    authorise_rule_scope(
        state,
        principal,
        input.kind.as_deref(),
        input.connector.as_deref(),
    )
    .await?;
    authorise_existing_rule(state, principal, id).await?;
    // BI-19 B1 review R6 (assistant path): the one place both the route and the
    // assistant's `update_alert_rule` pass through.
    authorise_digest_board(state, principal, Some(id), input).await?;
    // `SEC-50` slice 2 D7a (M7): the same place, for the tenant gate.
    gate_rule_statement(state, principal, headers, input).await
}

/// [`authorise_rule_scope`] plus the digest check for a rule being created:
/// the one guard the `POST /api/alerts` route and the assistant's
/// `create_alert_rule` both call (`BI-19` B1 review R6, assistant path).
///
/// # Errors
///
/// As [`authorise_rule_scope`] and `authorise_digest_board`.
pub(crate) async fn authorise_rule_create(
    state: &AppState,
    principal: Option<&Principal>,
    headers: &HeaderMap,
    input: &AlertRuleInput,
) -> Result<(), ApiError> {
    authorise_rule_scope(
        state,
        principal,
        input.kind.as_deref(),
        input.connector.as_deref(),
    )
    .await?;
    authorise_digest_board(state, principal, None, input).await?;
    // `SEC-50` slice 2 D7a (M7): the same place, for the tenant gate.
    gate_rule_statement(state, principal, headers, input).await
}

/// `SEC-50` slice 2 D7a (M7): a mart or metric rule's statement is gated as
/// the person SAVING it, with `SEC-20`'s 404, before the row is written.
///
/// Why it matters: the scheduled run acts as the deployment's catalog tenant
/// (D3), so without this a member of another tenant holding `alert:write`
/// could save a rule over `serving.*` and be delivered the catalog's number
/// at the target they chose. The gate is the person's own reach — the same
/// question Query Studio and every dashboard path already ask.
///
/// The statement is built by `lakehouse_alerts::statement_for_input`, which
/// normalizes with the save's own `normalize_input` and then calls the very
/// builders the run calls (`mart_statement`/`metric_statement`): one builder,
/// no second copy (`AGENTS.md` rule 4). A rule that reads no table of its own
/// (digest, freshness, connector, pipeline, chart) has nothing to gate.
///
/// The actor is the one every signed-in site of this slice uses: the person
/// as themselves, the request's `X-Tenant` honoured — and a job with no user
/// as the deployment's catalog tenant (`SEC-50` R4). The build's own errors
/// are classified exactly as `save_rule`'s callers classify them, so a rule
/// that could not be saved anyway reads the same here as there.
///
/// # Errors
///
/// The gate's own: 404 with `SEC-20`'s sentence for a table the saving person
/// may not read; 422 with D6's sentence for a job in a deployment with
/// several tenants and no catalog tenant; 503 when the deployment cannot
/// count its tenants. The builder's errors, classified as the save's are.
async fn gate_rule_statement(
    state: &AppState,
    principal: Option<&Principal>,
    headers: &HeaderMap,
    input: &AlertRuleInput,
) -> Result<(), ApiError> {
    let Some(sql) = lakehouse_alerts::statement_for_input(&state.clickhouse, input)
        .await
        .map_err(|err| {
            upstream_error::alert(&upstream_error::DATABASE, &err)
                .into_api_error(upstream_error::FailedAs::BadRequest)
        })?
    else {
        return Ok(());
    };
    crate::routes::query::StatementGate::clickhouse(
        state,
        crate::routes::gold::gold_export_gate_actor(principal, headers),
    )
    .check(&sql)
    .await
}

/// The check on a stored rule that an update or a delete acts on (`SRC-7`
/// D7; review BLOCKER 2 for delete): a rule of the six kinds must be one the
/// caller could have created, [`authorise_rule_scope`] on what is stored. An
/// unrestricted caller may touch any, so the rule is not even read for them;
/// an unknown id passes (the write is then a no-op or a validation error, as
/// before).
///
/// # Errors
///
/// As [`authorise_rule_scope`]; 503 with a fixed message when the stored
/// rule cannot be read.
pub(crate) async fn authorise_existing_rule(
    state: &AppState,
    principal: Option<&Principal>,
    id: &str,
) -> Result<(), ApiError> {
    if principal.is_some_and(crate::routes::catalog::is_unrestricted) {
        return Ok(());
    }
    let existing = lakehouse_alerts::get_rule(&state.clickhouse, id)
        .await
        .map_err(|err| {
            tracing::warn!(%err, "an alert rule could not be read before a change");
            ApiError::Unavailable("the alert rules could not be read".to_owned())
        })?;
    match existing {
        Some(rule) => {
            authorise_rule_scope(
                state,
                principal,
                Some(rule.kind.as_str()),
                rule.connector.as_deref(),
            )
            .await
        }
        None => Ok(()),
    }
}

/// The refusal for a digest of a restricted dashboard (owner decision Q-R12).
const RESTRICTED_DIGEST: &str =
    "A restricted dashboard's digest cannot be sent outside the console, so it cannot be saved.";

/// `BI-19` B1 review R6 and owner decision Q-R12: a `digest` rule reads a
/// dashboard and sends it to a mailbox or webhook, so saving one needs the
/// dashboard visible to the caller (the same 404 as an unknown one otherwise)
/// and not restricted ("only the people listed", itself or through a folder
/// above it): the digest would go to people the rules do not name. Fails
/// closed: a store that cannot answer refuses the save.
async fn authorise_digest_board(
    state: &AppState,
    principal: Option<&Principal>,
    id: Option<&str>,
    input: &AlertRuleInput,
) -> Result<(), ApiError> {
    let stored = match id {
        Some(id) => lakehouse_alerts::get_rule(&state.clickhouse, id)
            .await
            .map_err(|err| {
                tracing::warn!(%err, "an alert rule could not be read before a change");
                ApiError::Unavailable("the alert rules could not be read".to_owned())
            })?,
        None => None,
    };
    let is_digest = match input.kind.as_deref() {
        Some(kind) => kind == "digest",
        None => stored.as_ref().is_some_and(|r| r.kind == AlertKind::Digest),
    };
    if !is_digest {
        return Ok(());
    }
    let board = input
        .board
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .or_else(|| stored.as_ref().and_then(|r| r.board.as_deref()))
        .unwrap_or_default();
    if board.is_empty() || board == lakehouse_bi::store::DEFAULT_BOARD_ID {
        return Ok(());
    }
    let Some(principal) = principal else {
        return Err(ApiError::unauthorized());
    };
    let viewer = crate::routes::dashboard_access::viewer_for(state, principal).await?;
    let seen = lakehouse_bi::store::get_board(&state.clickhouse, board, &viewer)
        .await
        .map_err(|err| crate::routes::dashboard_folders::classify_ch_error(&err))?;
    if seen.is_none() {
        return Err(ApiError::NotFound(
            crate::routes::dashboard_organise::BOARD_NOT_FOUND.to_owned(),
        ));
    }
    let restricted = crate::routes::dashboard_access::board_is_restricted(
        state.pg.as_deref(),
        &state.clickhouse,
        board,
    )
    .await
    .unwrap_or(true);
    if restricted {
        return Err(ApiError::BadRequest(RESTRICTED_DIGEST.to_owned()));
    }
    Ok(())
}

/// The one save path for a rule (`BI-37` D1): `POST`/`PUT /api/alerts` and the
/// assistant's `create_alert_rule`/`update_alert_rule` all call it, so the scope
/// checks, the chart-alert save guard, the state reset and the store call
/// cannot drift apart (rule 4). Before this, the two tools called the guards
/// and `save_rule` themselves, which is how they skipped
/// [`crate::routes::chart_alerts::authorise_save`].
///
/// `id` is the rule being replaced (`None` for a new one). `input` gains its
/// owner from the save guard, and its chart fields are the ones the guard
/// judged.
///
/// `headers` are the request's, and are passed on for `SEC-50` slice 2 D7a's
/// statement gate (inside [`authorise_rule_create`]/[`authorise_rule_update`])
/// so the gate runs as the person SAVING, with their `X-Tenant`. The
/// assistant's tools pass an empty map: a tool call carries the model's
/// arguments, not the chat's headers, so the person's first tenant acts (the
/// same fallback `gold_export_gate_actor` documents).
///
/// # Errors
///
/// As [`authorise_rule_create`] / [`authorise_rule_update`] and
/// [`crate::routes::chart_alerts::authorise_save`]; 400 with the store's own
/// sentence on a validation failure (see `lakehouse_alerts::save_rule`).
pub(crate) async fn save_rule_with_checks(
    state: &AppState,
    principal: Option<&Principal>,
    headers: &HeaderMap,
    id: Option<&str>,
    input: &mut AlertRuleInput,
) -> Result<AlertRule, ApiError> {
    let stored = if let Some(id) = id {
        authorise_rule_update(state, principal, headers, id, input).await?;
        crate::routes::chart_alerts::stored_rule(state, id).await?
    } else {
        authorise_rule_create(state, principal, headers, input).await?;
        None
    };
    // BI-19 part C (C3): the router's floor is "signed in"; the permission, the
    // chart and the owner are settled here.
    crate::routes::chart_alerts::authorise_save(state, principal, id, stored.as_ref(), input)
        .await?;
    let rule = lakehouse_alerts::save_rule(&state.clickhouse, &state.webhook_sender(), input, id)
        .await
        .map_err(|err| {
            upstream_error::alert(&upstream_error::DATABASE, &err)
                .into_api_error(upstream_error::FailedAs::BadRequest)
        })?;
    // BI-19 C review SF-3: a rule that now watches something else starts from
    // `ok`, so it neither announces a "back to normal" for a line it never
    // crossed nor misses a crossing of the new one. Renaming it or changing its
    // target keeps the state.
    if let Some(id) = id
        && stored
            .as_ref()
            .is_some_and(|old| lakehouse_alerts::watches_differ(old, &rule))
    {
        forget_state(state, id).await;
    }
    Ok(rule)
}

/// `POST /api/alerts` — create a rule.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body or a
/// validation failure (see `lakehouse_alerts::save_rule`) — matching the
/// `TypeScript`'s single `catch` around both. For the `SRC-7` rule kinds, a
/// 401, 403 or 404 from [`authorise_rule_scope`].
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let mut input = parse_body(&body)?;
    let caller = principal.as_ref().map(|Extension(p)| p);
    let rule = save_rule_with_checks(&state, caller, &headers, None, &mut input).await?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// `PUT /api/alerts` — update a rule (body must include `id`).
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] on an unparseable body, a missing
/// `id`, or a validation failure — matching the `TypeScript`. For the
/// `SRC-7` rule kinds, a 401, 403 or 404 from [`authorise_rule_update`].
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let mut input = parse_body(&body)?;
    let Some(id) = input.id.clone() else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    let caller = principal.as_ref().map(|Extension(p)| p);
    let rule = save_rule_with_checks(&state, caller, &headers, Some(&id), &mut input).await?;
    Ok(ApiJson(json!({ "ok": true, "rule": rule })))
}

/// Query parameters for `DELETE /api/alerts` (`?id=`).
#[derive(Debug, Deserialize)]
pub struct DeleteQuery {
    id: Option<String>,
}

/// `DELETE /api/alerts?id=` — soft-delete a rule.
///
/// A rule of the `SRC-7` kinds is deleted only by a caller who could have
/// saved it ([`authorise_existing_rule`]); otherwise any `alert:write`
/// holder could silence another tenant's failure alerts (`SRC-7` review
/// BLOCKER 2). Other kinds, and an unknown id, behave as before.
///
/// # Errors
///
/// Returns a 400 [`ApiError::BadRequest`] when `id` is missing, or a 500
/// [`ApiError::Internal`] on a `ClickHouse` failure — matching the
/// `TypeScript`'s pre-`try` `id` check (400) vs. its `catch` (500). For the
/// `SRC-7` kinds, a 401, 403 or 404 from [`authorise_existing_rule`].
pub async fn delete(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = query.id else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    let caller = principal.as_ref().map(|Extension(p)| p);
    authorise_existing_rule(&state, caller, &id).await?;
    // BI-19 part C (C3): the router's floor is "signed in"; without the alert
    // permission a person removes only their own chart alert.
    if !crate::routes::chart_alerts::may_manage(
        caller,
        crate::routes::chart_alerts::stored_rule(&state, &id)
            .await?
            .as_ref(),
    ) {
        return Err(ApiError::PermissionDenied("alert:write".to_owned()).into());
    }
    lakehouse_alerts::delete_rule(&state.clickhouse, &id)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::Internal,
            )
        })?;
    forget_state(&state, &id).await;
    Ok(ApiJson(json!({ "ok": true })))
}

/// Drop the stored state of rule `id` (`BI-19` part C, D11): after its delete,
/// and when an edit changes what it watches. The one place that does it, for the
/// route and for the assistant's `delete_alert_rule` (`BI-19` C review SF-7).
/// Best effort: a state that cannot be removed is logged, not an error to the
/// caller, because the rule itself is already changed.
pub(crate) async fn forget_state(state: &AppState, id: &str) {
    if let Some(pg) = state.pg.as_deref()
        && let Err(err) = lakehouse_store::alert_state::delete(pg, id).await
    {
        tracing::warn!(%err, rule_id = %id, "an alert's state could not be removed");
    }
}

/// Query parameters for `GET`/`POST /api/alerts/run`.
#[derive(Debug, Deserialize, Default)]
pub struct RunQuery {
    /// Run only this rule id, when given.
    id: Option<String>,
    /// Shared token, as a query-string fallback to the `x-run-token`
    /// header.
    token: Option<String>,
}

/// The `/api/alerts/run` guard, split out of the handler so it can be
/// exercised without a live `ClickHouse` connection or ever calling
/// `run_rules`. Was a straight port of `alerts/run/route.ts:16-20`; is now
/// (D4) fail-closed instead of fail-open — see the module-level "D4" doc
/// comment on [`run`] for the full rationale.
///
/// # Security property
///
/// * `configured: Some(token)` — unchanged from the `TypeScript`: the
///   caller's header/query token must match, or this is a 401.
/// * `configured: None` (`ALERTS_RUN_TOKEN` unset) — the `TypeScript`
///   (and this handler, pre-D4) skipped the check entirely, making the
///   route completely unauthenticated: any caller could trigger a real
///   evaluation run that sends real webhooks/emails. D4 closes that: with
///   no shared token configured, only a **service-identity** principal
///   (`PrincipalId::Service` — the cron/scheduler's own credential, not a
///   logged-in human's session) is allowed through; anyone/anything else
///   is refused with 503. A human session principal is deliberately NOT
///   sufficient here even though the router's `Policy::RequiresAuth`
///   already requires ONE — this guard's whole job is to stop an ordinary
///   authenticated user (any signed-up account, not just an operator) from
///   firing every registered alert channel just by hitting this URL.
fn check_run_token(
    configured: Option<&str>,
    header_token: Option<&str>,
    query_token: Option<&str>,
    principal: Option<&Principal>,
) -> Result<(), ApiError> {
    if let Some(need) = configured {
        return if header_token.or(query_token) == Some(need) {
            Ok(())
        } else {
            Err(ApiError::unauthorized())
        };
    }
    match principal {
        Some(p) if matches!(p.id, PrincipalId::Service(_)) => Ok(()),
        _ => Err(ApiError::Unavailable(
            "alerts run is not configured: set ALERTS_RUN_TOKEN, or call with \
             service-identity credentials (not a human user session)"
                .to_owned(),
        )),
    }
}

/// [`FreshnessSource`] backed by real Postgres `dataset_sla` rows and
/// WS2's Iceberg REST surface — the in-process path, not an HTTP call to
/// this crate's own `/api/lakehouse/tables/{ns}/{table}` route (which
/// returns `TableDetail`, a struct with no top-level `last_updated_ms`
/// field at all). `lakehouse_iceberg::rest::load_table_summary` returns
/// `TableSummary`, which does carry `last_updated_ms`, so this calls that
/// function directly, in the same process, rather than looping back
/// through HTTP. WS5 item C1.
pub(in crate::routes) struct ApiFreshnessSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
    pub(in crate::routes) iceberg: Arc<IcebergClient>,
}

#[async_trait::async_trait]
impl FreshnessSource for ApiFreshnessSource<'_> {
    async fn expected_interval_minutes(&self, table_name: &str) -> Result<Option<i64>, String> {
        // `StoreError`'s own `Display` renders the fixed string "database
        // error" (`lakehouse-store/src/error.rs`), never an upstream
        // detail, so `.to_string()` here is not a leak — unlike
        // `ChError`/`DgError`'s `Server` variant elsewhere in this crate.
        lakehouse_store::governance::expected_interval_minutes_for(self.pg, table_name)
            .await
            .map(|opt| opt.map(i64::from))
            .map_err(|err| err.to_string())
    }

    async fn last_snapshot_ms(&self, table_name: &str) -> Option<i64> {
        let (ns, table) = table_name.split_once('.')?;
        let ident = TableIdent::new(NamespaceIdent::new(ns.to_owned()), table.to_owned());
        lakehouse_iceberg::rest::load_table_summary(&self.iceberg, &ident)
            .await
            .ok()
            .and_then(|summary| summary.last_updated_ms)
    }
}

/// [`SilenceSource`] backed by real Postgres `alert_instance.silenced_until`
/// (`lakehouse_store::overview::is_rule_silenced`). WS5 item C1, WS5 plan
/// review U6.
pub(in crate::routes) struct ApiSilenceSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
}

#[async_trait::async_trait]
impl SilenceSource for ApiSilenceSource<'_> {
    async fn is_silenced(&self, rule_id: &str) -> bool {
        overview::is_rule_silenced(self.pg, rule_id, OffsetDateTime::now_utc())
            .await
            // A Postgres error here must not turn a legitimate alert into
            // a silent no-op that also fails to fire — degrade to "not
            // silenced," never fail the whole run.
            .unwrap_or(false)
    }
}

/// [`StateSource`] backed by Postgres `alert_rule_state` (`BI-19` part C, D11,
/// migration `0113`). A read that fails is the fixed sentence (principle 4), and
/// the rule is then skipped rather than judged; a write that fails is logged
/// here, because the caller has already sent or not sent.
pub(crate) struct ApiStateSource<'a> {
    pub(crate) pg: &'a PgPool,
}

#[async_trait::async_trait]
impl StateSource for ApiStateSource<'_> {
    async fn state_of(&self, rule_id: &str) -> Result<AlertState, String> {
        match lakehouse_store::alert_state::get(self.pg, rule_id).await {
            Ok(row) => Ok(row.map_or(AlertState::Ok, |r| AlertState::parse(&r.state))),
            Err(err) => {
                tracing::warn!(%err, rule_id, "an alert's state could not be read");
                Err("the alert's state could not be read.".to_owned())
            }
        }
    }

    async fn record(
        &self,
        rule_id: &str,
        state: AlertState,
        value: Option<f64>,
        reason: Option<&str>,
    ) {
        if let Err(err) = lakehouse_store::alert_state::record_check(
            self.pg,
            rule_id,
            state.as_str(),
            value,
            reason,
            OffsetDateTime::now_utc(),
        )
        .await
        {
            tracing::warn!(%err, rule_id, "an alert's state could not be saved");
        }
    }

    async fn claim(
        &self,
        rule_id: &str,
        previous: AlertState,
        next: AlertState,
        value: Option<f64>,
    ) -> Result<bool, String> {
        lakehouse_store::alert_state::claim(
            self.pg,
            rule_id,
            previous.as_str(),
            next.as_str(),
            value,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|err| {
            tracing::warn!(%err, rule_id, "an alert's transition could not be claimed");
            "the alert's state could not be saved, so it was not sent.".to_owned()
        })
    }

    async fn release(&self, rule_id: &str, claimed: AlertState, previous: AlertState) {
        if let Err(err) = lakehouse_store::alert_state::release(
            self.pg,
            rule_id,
            claimed.as_str(),
            previous.as_str(),
            OffsetDateTime::now_utc(),
        )
        .await
        {
            tracing::warn!(%err, rule_id, "a claimed alert transition could not be given back");
        }
    }

    async fn record_failed(&self, rule_id: &str, reason: &str) {
        if let Err(err) = lakehouse_store::alert_state::record_failed_check(
            self.pg,
            rule_id,
            reason,
            OffsetDateTime::now_utc(),
        )
        .await
        {
            tracing::warn!(%err, rule_id, "an alert's failed check could not be saved");
        }
    }
}

/// [`SqlGate`] over the shared policy engine
/// (`policy_engine::rewrite_sql_for_roles`, the path tiles, embeds and
/// Query Studio use).
///
/// `BI-39` D1: a rule saved now reads as the person who saved it — their roles,
/// masking and row filters, and the principal/tenant placeholders — the same
/// rewrite a dashboard read as that person gets. A rule saved before owners
/// existed carries none, and keeps the least-privileged "Dashboard Viewer" role
/// it always had, the mapping `embed.rs` documents for public dashboards (`BI-39`
/// D4); a digest passes no owner for the same reason (`BI-39` D1, out of scope).
/// A refusal is the engine's fixed, non-leaking message.
///
/// `SEC-50` slice 2 T7 (M1): the TENANT gate runs first, through the same
/// [`crate::routes::query::StatementGate`] every other slice-2 site asks —
/// an alert is one more way to read the shared catalog, and it must not
/// read it from a tenant that may not (`SEC-20`). The actor is the one
/// `SEC-50` names for the path that built this gate: the deployment's
/// catalog tenant for `/api/alerts/run` (D3), the calling person for the
/// assistant's `run_alert_rule` (M6, R4).
///
/// The state, not a `pg`/`ch` pair: the gate needs the pool
/// (`CATALOG_TENANT_ID`, the tenant count) as well as the engine, and
/// `SEC-50` slice 2 M1 is exactly that the sites carry the whole state.
pub(in crate::routes) struct ApiSqlGate<'a> {
    pub(in crate::routes) state: &'a AppState,
    pub(in crate::routes) actor: crate::routes::query::GateActor<'a>,
}

/// What the gate answers for an owner whose record cannot be read (`BI-39` D5):
/// a check that could not run, so the rule's state is left alone (C2) and the
/// sentence is recorded.
const OWNER_UNREADABLE: &str = "The person who set this alert could not be read.";

/// Plan 1f: [`LateSource`] backed by Postgres (for the SLA row) and a
/// pre-fetched map of last-success epoch seconds per pipeline id
/// (populated by the route handler from the Dagster client, so this
/// crate stays free of the Dagster SDK). When Postgres is not
/// configured, or when either query fails, the error message is
/// classified ("database error") before returning — never an upstream
/// detail, matching AGENTS.md rule 4.
pub(in crate::routes) struct ApiLateSource<'a> {
    pub(in crate::routes) pg: &'a PgPool,
    /// `pipeline_id → last successful run's epoch seconds`. A *missing*
    /// key means "no SUCCESS run on record yet" — the [`late`] helper
    /// treats that as "definitely late, threshold set." The run id is
    /// stored alongside by [`evaluate_late_pass`] for the
    /// `pipeline_run_event` dedupe; this source only carries the epoch
    /// seconds because [`LateSource::late_inputs`] is the only method
    /// the alerts crate's evaluator calls.
    pub(in crate::routes) last_success: HashMap<String, Option<LastSuccess>>,
}

#[async_trait::async_trait]
impl LateSource for ApiLateSource<'_> {
    async fn late_inputs(&self, pipeline_id: &str) -> Result<Option<(i32, Option<f64>)>, String> {
        // SLA row first: without a threshold the answer is "config gap",
        // not "not late". `get_pipeline_sla` already classifies its
        // error → `Ok(None)` for missing rows, `Err("database error")`
        // for transport failures.
        let sla = match lakehouse_store::pipelines::get_pipeline_sla(self.pg, pipeline_id).await {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(None),
            Err(_) => return Err("database error".to_owned()),
        };
        let threshold = match sla.late_after_seconds {
            Some(t) if t > 0 => t,
            // `late_after_seconds = NULL` (only `max_duration_seconds`
            // set) — there is no "late" claim to make for this
            // pipeline.
            _ => return Ok(None),
        };
        // The route handler pre-fetches last-success epoch seconds
        // per pipeline. A missing key means the Dagster round-trip
        // didn't include this pipeline id — treat it as "no SUCCESS
        // run on record," not as an error: that's the same posture a
        // missing key would have if Dagster had no runs for the job.
        let last = self
            .last_success
            .get(pipeline_id)
            .and_then(|o| o.as_ref())
            .map(|s| s.epoch_seconds);
        Ok(Some((threshold, last)))
    }
}

#[async_trait::async_trait]
impl SqlGate for ApiSqlGate<'_> {
    async fn gate(&self, owner: Option<&str>, sql: &str) -> Result<String, GateRefusal> {
        // `SEC-50` slice 2 T7 (M1/M2): the tenant gate first, before the
        // rewrite and before the engine. A refusal keeps the gate's own
        // fixed sentence (never upstream text, `AGENTS.md` principle 4) as
        // the `Err`, which the alerts crate records as the rule's failed
        // check (`lakehouse-alerts` `lib.rs`, `run_one` → `settle`) and
        // then goes on with the next rule — one tenant's refusal must not
        // stop the deployment's other rules.
        crate::routes::query::StatementGate::clickhouse(self.state, self.actor)
            .check(sql)
            .await
            .map_err(|err| GateRefusal::Refused(err.to_string()))?;
        let Some(owner) = owner else {
            // `BI-39` D4: a rule saved before owners existed reads as the role
            // every alert used to, and a digest keeps it (`BI-39` D1).
            return self
                .rewrite(
                    sql,
                    &[crate::routes::embed::EMBED_VIEWER_ROLE.to_owned()],
                    &crate::sql_rewrite::PlaceholderValues::none(),
                )
                .await;
        };
        // `BI-39` D1/D5: the rule reads as its owner, and a person who cannot
        // be read stops it rather than falling back to a wider or narrower
        // view. `principal_of` is the reader the PDF, the reports and chart
        // alerts already use — one copy (rule 4).
        let principal = match crate::routes::render::principal_of(self.state, owner).await {
            Ok(Some(principal)) => principal,
            Ok(None) => return Err(GateRefusal::Stop(lakehouse_alerts::OWNER_GONE)),
            Err(_) => return Err(GateRefusal::Refused(OWNER_UNREADABLE.to_owned())),
        };
        // `BI-39` D5: the permission is what allowed a rule to send to somebody
        // other than its owner (feature C3), so a rule does not outlive it. A
        // rule sent only to the owner's own address does not need it, and such
        // a rule is the chart kind — read by `routes::chart_alerts`, never
        // through this gate — because a mart or metric rule is only ever saved
        // by somebody holding `alert:write` (`chart_alerts::authorise_save`).
        if !principal.has("alert:write") {
            return Err(GateRefusal::Stop(lakehouse_alerts::OWNER_NO_PERMISSION));
        }
        let placeholders = crate::sql_rewrite::PlaceholderValues {
            principal_id: Some(principal.id.uuid().to_string()),
            principal_tenant_ids: principal
                .tenant_ids
                .iter()
                .map(ToString::to_string)
                .collect(),
        };
        self.rewrite(sql, &principal.role_names, &placeholders)
            .await
    }

    async fn board_restricted(&self, board_id: &str) -> bool {
        // BI-19 B1 review R6: fail closed, a store that cannot answer counts as
        // restricted (the digest is skipped, not sent).
        crate::routes::dashboard_access::board_is_restricted(
            self.state.pg.as_deref(),
            &self.state.clickhouse,
            board_id,
        )
        .await
        .unwrap_or(true)
    }
}

impl ApiSqlGate<'_> {
    /// `sql` rewritten for exactly `roles` and `placeholders` — the one call to
    /// the shared policy engine, so the owner's path and the legacy path cannot
    /// drift apart (rule 4).
    async fn rewrite(
        &self,
        sql: &str,
        roles: &[String],
        placeholders: &crate::sql_rewrite::PlaceholderValues,
    ) -> Result<String, GateRefusal> {
        let obligations = crate::policy_engine::PolicyEngineObligations::new(
            self.state.pg.as_deref(),
            &self.state.clickhouse,
        );
        crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::ClickHouseDialect {},
            crate::sql_rewrite::Engine::ClickHouse,
            roles,
            placeholders,
            &obligations,
        )
        .await
        // GOV-7 D5: the run history records the not-found sentence itself,
        // not a generic failure, for a table the owner may not read.
        .map_err(|err| {
            GateRefusal::Refused(crate::routes::support::enforcement_refusal(sql, &err).to_string())
        })
    }
}

/// A fixed, kind-derived `AlertItem.source` label for a fired rule's
/// persisted instance — never invented per-rule text, just which engine
/// produced it.
fn fired_source(kind: AlertKind) -> &'static str {
    match kind {
        AlertKind::Alert => "Alert rules",
        AlertKind::Freshness => "Freshness monitoring",
        AlertKind::Digest => "Digest", // never reached: callers filter Digest out before this
        AlertKind::PipelineFailure => "Pipeline failures", // never reached: callers filter PipelineFailure out before this
        AlertKind::PipelineSlow => "Pipeline SLA: duration",
        AlertKind::PipelineLate => "Pipeline SLA: late",
        AlertKind::PipelineVolumeDrop => "Pipeline SLA: volume drop",
        AlertKind::PipelineBehindTarget => "Pipeline target lag: behind",
        // `SRC-7`: connector and upload rules fire from the run event
        // routes (`routes::pipelines`), which persist their own instance
        // with the source label; `run_rules` skips them, so these labels
        // are only a fallback for an exhaustive match.
        AlertKind::ConnectorFailure
        | AlertKind::ConnectorRepeatedFailure
        | AlertKind::ConnectorDisabled
        | AlertKind::ConnectorSchemaChange
        | AlertKind::ConnectorSuccess => "Connector runs",
        AlertKind::ConnectorLag => "Change capture",
        AlertKind::UploadFailure => "File uploads",
        // `DATA-14`: delivered when an incident opens
        // (`routes::quality_incidents`), which persists its own instance;
        // `run_rules` skips the kind, so this is only an exhaustive-match label.
        AlertKind::QualityIncident => "Data quality",
    }
}

/// A human-readable description of a fired rule's breach, built from
/// [`lakehouse_alerts::RunResult::value`] and the rule's own config —
/// never fabricated beyond what both already carry.
fn fired_detail(rule: &AlertRule, value: Option<f64>) -> String {
    let value = value.map_or_else(|| "n/a".to_owned(), |v| format!("{v:.2}"));
    match rule.kind {
        AlertKind::Freshness => format!(
            "{} is stale: observed age {value} min",
            rule.mart.as_deref().unwrap_or("")
        ),
        // BI-19 part C (planner decision 3): `alert_instance` is one list for
        // everyone, so a chart alert's detail never carries the value.
        AlertKind::Alert if rule.chart.is_some() => format!(
            "chart alert: the chart is {} its line",
            match rule.condition.as_deref() {
                Some("below") => "below",
                Some("any_rows") => "returning rows against",
                _ => "above",
            }
        ),
        // BI-2: a rule that watches a metric says so, not a column.
        AlertKind::Alert if rule.metric.is_some() => format!(
            "metric {} {} {} — observed {value}",
            rule.metric.as_deref().unwrap_or(""),
            rule.op.as_str(),
            rule.threshold
        ),
        AlertKind::Alert | AlertKind::Digest => format!(
            "{}({}) on {} {} {} — observed {value}",
            rule.agg,
            rule.measure.as_deref().unwrap_or(""),
            rule.mart.as_deref().unwrap_or(""),
            rule.op.as_str(),
            rule.threshold
        ),
        // PipelineFailure / PipelineSlow / PipelineVolumeDrop rules fire
        // from the event routes, never `run_rules`; the detail isn't
        // used for them.
        AlertKind::PipelineFailure => "pipeline failure".to_owned(),
        AlertKind::PipelineSlow => "pipeline run exceeded its duration SLA".to_owned(),
        AlertKind::PipelineLate => {
            "pipeline is late (no successful run within the SLA window)".to_owned()
        }
        AlertKind::PipelineVolumeDrop => {
            "pipeline run processed fewer than half the median rows of prior runs".to_owned()
        }
        AlertKind::PipelineBehindTarget => "pipeline is behind its target lag".to_owned(),
        // `SRC-7`: never reached from `run_rules` (it skips these kinds);
        // the event routes write their own detail.
        AlertKind::ConnectorFailure
        | AlertKind::ConnectorRepeatedFailure
        | AlertKind::ConnectorDisabled
        | AlertKind::ConnectorSchemaChange
        | AlertKind::ConnectorSuccess
        | AlertKind::ConnectorLag
        | AlertKind::UploadFailure => "connector or upload event".to_owned(),
        // `DATA-14`: never reached from `run_rules`; the incident route writes its own.
        AlertKind::QualityIncident => "quality incident".to_owned(),
    }
}

/// Persist every fired, non-`Digest` result from `results` as an
/// `alert_instance` row, best-effort — mirrors `routes::query.rs:171-198`'s
/// history write: a Postgres outage (or a `ClickHouse` re-fetch failure)
/// must never turn an otherwise-successful `/api/alerts/run` into an error
/// response, since the webhook/email already went out. Skips (never
/// inserts) a rule the caller's own [`ApiSilenceSource`] reports as
/// currently silenced — the route layer's decision, not
/// `insert_from_fired_rule`'s (that primitive has no silence awareness by
/// design; see `lakehouse_store::overview`'s module doc comment). WS5 item
/// C1, WS5 plan review U6.
async fn persist_fired_results(
    pg: &PgPool,
    ch: &lakehouse_clickhouse::ChClient,
    results: &[lakehouse_alerts::RunResult],
) {
    if !results
        .iter()
        .any(|r| r.fired && r.kind != AlertKind::Digest)
    {
        return;
    }
    // A second `list_rules` round trip, deliberately: `RunResult` carries
    // only id/name/kind/fired/value, never the rule's own `severity`/
    // `mart` this insert needs, and `run_rules` does not return its
    // internal rule list.
    let rules = match lakehouse_alerts::list_rules(ch).await {
        Ok(rules) => rules,
        Err(err) => {
            tracing::warn!(%err, "failed to re-fetch alert rules for alert_instance persistence (results were still delivered)");
            return;
        }
    };
    let now = OffsetDateTime::now_utc();
    for result in results {
        if !result.fired || result.kind == AlertKind::Digest {
            continue;
        }
        let Some(rule) = rules.iter().find(|r| r.id == result.id) else {
            continue;
        };
        let silenced = ApiSilenceSource { pg }.is_silenced(&rule.id).await;
        if silenced {
            continue;
        }
        // BI-19 C review SF-2: a delivery that keeps failing is tried again at
        // every check; the crossing keeps the one open instance it has (a retry
        // that arrives updates it) instead of adding one per check.
        if rule.kind == AlertKind::Alert
            && let Some(delivery) = result
                .delivered
                .as_ref()
                .and_then(crate::routes::load_alerts::delivery_record)
        {
            match overview::retry_open_undelivered(pg, &rule.id, delivery).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(err) => {
                    tracing::warn!(%err, rule_id = %rule.id, "an open alert instance could not be checked for a retry");
                }
            }
        }
        let detail = fired_detail(rule, result.value);
        let fired = FiredRule {
            rule_id: &rule.id,
            title: &rule.name,
            severity: rule.severity.as_deref(),
            source: fired_source(rule.kind),
            affected: rule
                .mart
                .as_deref()
                .or(rule.metric.as_deref())
                .unwrap_or(""),
            detail: &detail,
        };
        // `SRC-17` F1: the instance keeps whether the delivery arrived (the
        // result's `error` is a fixed sentence, never upstream text).
        let delivery = result
            .delivered
            .as_ref()
            .and_then(crate::routes::load_alerts::delivery_record);
        if let Err(err) =
            overview::insert_from_fired_rule_with_delivery(pg, &fired, delivery, now).await
        {
            tracing::warn!(%err, rule_id = %rule.id, "failed to persist fired alert instance (delivery already happened)");
        }
    }
}

/// A rule that is back under its line closes the instances it opened
/// (`BI-19` part C, D12), so the alert list shows one state for one condition.
/// Best effort, like the insert: the message has already been sent.
async fn resolve_recovered(pg: &PgPool, results: &[lakehouse_alerts::RunResult]) {
    for result in results.iter().filter(|r| r.recovered) {
        if let Err(err) = lakehouse_store::alert_state::resolve_open_instances(
            pg,
            &result.id,
            "The value is back within the line.",
        )
        .await
        {
            tracing::warn!(%err, rule_id = %result.id, "recovered alert instances could not be resolved");
        }
    }
}

/// Run the alert rules once and deliver what fires (`BI-40` D4): the one place
/// the rule part of a run lives, shared by the scheduler's tick
/// ([`run`]), a person's test of one rule
/// (`crate::routes::alerts::test_one`) and the assistant's `run_alert_rule`.
///
/// `only` narrows the run to one rule id, as `?id=` does. The wiring is
/// deliberately the route's whole rule part and nothing of the scheduler's
/// extra passes (quality, schema versions, semantic drafting, lineage,
/// pipeline-late): those belong to the tick and are not a test of one rule.
///
/// - `run_rules` evaluates the threshold, digest, freshness and pipeline kinds;
/// - a chart rule is answered `CHART_SKIPPED` there, so
///   [`crate::routes::chart_alerts::evaluate`] reads it as its owner and its
///   result replaces the skipped one (`BI-19` D12);
/// - every fired, non-`Digest` result is saved as an `alert_instance` and every
///   recovery resolves its open instance (`BI-19` D12), best-effort.
///
/// `actor` is the caller the gate measures each rule's statement against
/// (`SEC-50` slice 2 T7, M6/R4): the deployment's catalog tenant for the
/// scheduler's tick, the signed-in person for a test run and for the
/// assistant's `run_alert_rule`. The caller-of-this-helper decides, so no path
/// can run a rule around the gate.
///
/// # Errors
///
/// 500 [`ApiError::Internal`] on a `ClickHouse` failure while listing or
/// evaluating rules; the fixed "database error" sentence, never upstream text.
pub(crate) async fn run_one(
    state: &AppState,
    only: Option<&str>,
    actor: crate::routes::query::GateActor<'_>,
) -> Result<Vec<lakehouse_alerts::RunResult>, ApiError> {
    // SEC-10: the sender checks every webhook target and never follows a redirect.
    let http = state.webhook_sender();
    let email = EmailSender::new(crate::runtime_settings::email(state).await.smtp());
    // `freshness`/`silence` sources are real, Postgres/Iceberg-backed
    // implementations when both dependencies are configured for this
    // request; otherwise `None` — every `Freshness` rule then reports
    // `skipped` and no rule's delivery is ever silence-suppressed, an
    // honest degrade rather than a placeholder failure. `AppState::iceberg`
    // is read once, through the lock, with the guard dropped immediately
    // (never held across an `.await`) — this only uses whatever client is
    // already cached; it does not itself trigger a connect.
    let cached_iceberg = state.iceberg.read().await.clone();
    let freshness_source = match (state.pg.as_deref(), cached_iceberg) {
        (Some(pg), Some(iceberg)) => Some(ApiFreshnessSource { pg, iceberg }),
        _ => None,
    };
    let silence_source = state.pg.as_deref().map(|pg| ApiSilenceSource { pg });
    let state_source = state.pg.as_deref().map(|pg| ApiStateSource { pg });
    // `SEC-50` slice 2 T7 (M1): the gate is the caller's, carried whole — the
    // state holds the pool the tenant count reads, the actor says whose reach
    // the rule's statement is measured against.
    let gate = ApiSqlGate { state, actor };

    let mut results = lakehouse_alerts::run_rules(
        &state.clickhouse,
        &http,
        &email,
        only,
        freshness_source.as_ref().map(|s| s as &dyn FreshnessSource),
        silence_source.as_ref().map(|s| s as &dyn SilenceSource),
        state_source.as_ref().map(|s| s as &dyn StateSource),
        &gate,
    )
    .await
    .map_err(|err| {
        upstream_error::ch_error_as(
            &upstream_error::DATABASE,
            &err,
            upstream_error::FailedAs::Internal,
        )
    })?;

    // BI-19 part C (D12): chart rules are read as their owners here; their
    // `run_rules` entries (skipped, with the reason) are replaced by the result.
    let state_ref = state_source.as_ref().map(|s| s as &dyn StateSource);
    let chart_results = crate::routes::chart_alerts::evaluate(
        state,
        (&http, &email),
        silence_source.as_ref().map(|s| s as &dyn SilenceSource),
        state_ref,
        only,
    )
    .await;
    results.retain(|r| !chart_results.iter().any(|c| c.id == r.id));
    results.extend(chart_results);

    // Persist every fired, non-Digest result as an `alert_instance` row,
    // best-effort (WS5 item C1) — see [`persist_fired_results`]'s doc
    // comment.
    if let Some(pg) = state.pg.as_deref() {
        persist_fired_results(pg, &state.clickhouse, &results).await;
        resolve_recovered(pg, &results).await;
    }
    Ok(results)
}

/// `GET`/`POST /api/alerts/run` — evaluate rules and deliver alerts/
/// digests that fire.
///
/// # Warning
///
/// This is a live, side-effecting endpoint: on success it queries
/// `serving.*` marts and, for every rule that fires, sends a real webhook
/// `POST` or a real `SMTP` email. It is NOT captured in the parity corpus
/// for exactly this reason — see `rust/tests/parity/README.md`'s
/// "Deliberate omissions" section. Do not call this against production
/// infrastructure while testing; only the rejection paths (wrong/missing
/// token with `ALERTS_RUN_TOKEN` set; no token configured and no
/// service-identity principal) are safe to exercise.
///
/// # D4: fail closed when `ALERTS_RUN_TOKEN` is unset
///
/// The `TypeScript` original — and this handler, before this fix — skipped
/// its shared-token check entirely when `ALERTS_RUN_TOKEN` was unset,
/// which is precisely the state of this environment: the guard was a
/// no-op, and any caller (any signed-up user, once auth landed; literally
/// anyone, before it) could trigger every alert rule and fire real
/// webhooks/emails. [`check_run_token`] now fails closed: with no token
/// configured, only a `PrincipalId::Service` principal — a
/// `service_identity` credential meant for the cron/scheduler that runs
/// this on a timer, not a human's browser session — is let through;
/// everyone else gets a 503. The 503 (not 401) is deliberate: this is a
/// missing-configuration state ("nobody set up how this route should be
/// called"), the same idiom `routes::identity::pool` uses for "no
/// `DATABASE_URL`" — not a bad-credential state, which is what 401 means
/// everywhere else in this crate. The token path (`ALERTS_RUN_TOKEN` set)
/// is unchanged, so an existing cron/scheduler integration keeps working
/// exactly as before.
///
/// # Errors
///
/// Returns a 401 [`ApiError::unauthorized`] when `ALERTS_RUN_TOKEN` is set
/// and the caller's token doesn't match; a 503 [`ApiError::Unavailable`]
/// when it is unset and the caller is not an authenticated service-identity
/// principal; or a 500 [`ApiError::Internal`] on a `ClickHouse` failure
/// while listing rules.
pub async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<RunQuery>,
    principal: Option<Extension<Principal>>,
) -> ApiResult<ApiJson<Value>> {
    let header_token = headers.get("x-run-token").and_then(|v| v.to_str().ok());
    check_run_token(
        state.config.alerts_run_token.as_deref(),
        header_token,
        query.token.as_deref(),
        principal.as_ref().map(|Extension(p)| p),
    )?;

    // `SEC-50` slice 2 T7 (D3): this route is the scheduled run — a run token
    // or a service identity (`check_run_token`), never a person's browser
    // session — so it acts as the deployment's catalog tenant.
    let results = run_one(
        &state,
        query.id.as_deref(),
        crate::routes::query::GateActor::CatalogTenant,
    )
    .await?;

    // The scheduler's passes below need the same senders and silence source the
    // rule part used; `run_one` keeps its own, so this is a second, cheap build
    // of the same configuration (`BI-40` D4).
    // SEC-10: the sender checks every webhook target and never follows a redirect.
    let http = state.webhook_sender();
    let email = EmailSender::new(crate::runtime_settings::email(&state).await.smtp());
    let silence_source = state.pg.as_deref().map(|pg| ApiSilenceSource { pg });

    // Plan 1f: `pipeline_late` rules fire from this op, not from the
    // orchestrator event path. A "late" claim depends on the clock at
    // evaluation time, not at run time. The route handler iterates
    // every enabled `pipeline_late` rule whose `pipeline` names a
    // specific pipeline id (the `*` wildcard is not yet wired for
    // pipeline_late — it would require enumerating every runnable
    // pipeline here, which is the plan's next-step scope, not this
    // commit's) and calls [`evaluate_pipeline_late`] per pipeline.
    // The matching `run_rules` entries (each a `skipped` with the
    // reason "pipeline_late rules are evaluated by /api/alerts/run...")
    // stay in the response, so the user sees both the rule list and
    // the per-pipeline deliveries.
    if let Some(pg) = state.pg.as_deref() {
        let late_rule_pipelines = list_late_rule_pipelines(&state.clickhouse).await?;
        if !late_rule_pipelines.is_empty() {
            let last_success =
                fetch_last_success_per_pipeline(&state.dagster, &late_rule_pipelines).await;
            // Plan 1f (reviewer fix #4): the clock helper returns `None`
            // when the system clock is unset. Pass that `None` straight
            // through to `evaluate_late_pass` — it skips every pipeline
            // (a "late" claim is undecidable without a clock) rather
            // than silently fabricating a `Some(0.0)` "late since 1970"
            // for each one. The helper log line is the visible audit
            // trail for the skip.
            let now_seconds = crate::routes::pipelines::now_unix_seconds();
            evaluate_late_pass(
                pg,
                &state.clickhouse,
                &http,
                &email,
                late_rule_pipelines,
                &last_success,
                silence_source.as_ref().map(|s| s as &dyn SilenceSource),
                now_seconds,
            )
            .await?;
        }
    }

    // The same tick keeps quality verdicts current: every runnable rule
    // whose verdict is over an hour old is run again, in the background —
    // a table scan must not hold up this response (the scheduler waits 30
    // seconds for it). Not on a single-rule run (`?id=`), which is someone
    // testing one alert, not the tick.
    let quality_started =
        query.id.is_none() && crate::routes::quality::spawn_scheduled_pass(&state);

    // And records the schema a Silver or Gold table has now when it differs
    // from the last one recorded (ADR 0015), in the background for the same
    // reason, and not on a single-rule run either.
    let schema_started = query.id.is_none() && crate::routes::schema_versions::spawn_pass(&state);

    // The semantic layer's drafting pass (AI-16): asks the deployment's
    // model for a description of each table that has none, up to ten per
    // tick, in the background for the same reason. The response does not
    // report it: it changes no verdict, and a pass that starts nothing
    // (switch off, no key, one already running) is not a fault to show.
    if query.id.is_none() {
        crate::routes::ai::semantic::spawn_pass(&state);
        // DATA-13 (F3): the builds `ClickHouse` logged since the last tick
        // become column lineage, in the background for the same reason.
        crate::routes::lineage::columns::spawn_builds_pass(&state);
    }

    Ok(ApiJson(json!({
        "ran": results.len(),
        "results": results,
        "qualityPassStarted": quality_started,
        "schemaPassStarted": schema_started,
    })))
}

/// Query parameters for `POST /api/alerts/test`.
#[derive(Debug, Deserialize)]
pub struct TestQuery {
    /// The rule to run, required.
    id: Option<String>,
}

/// The answer of a `pipeline_late` rule asked to test-run (`BI-40` D3, honest:
/// principle 2). A "late" claim needs the Postgres SLA row and the
/// orchestrator's last success, so it belongs to the 15-minute tick, not to a
/// single-rule test.
const PIPELINE_LATE_TEST: &str =
    "A late-pipeline alert is checked by the 15-minute pass, not by a test run.";

/// The one refusal an unknown rule and a rule the caller cannot see share, so
/// the route is not an oracle (`BI-40` D2.1).
const RULE_NOT_FOUND: &str = "alert rule not found.";

/// The guard of `POST /api/alerts/test` and the assistant's `run_alert_rule`
/// (`BI-40` D2, D7): who may run this one rule now.
///
/// All three must hold, in this order:
///
/// 1. the rule is in what the caller's list shows ([`visible_rules`] over the
///    one rule), else 404 — the same body an unknown id gets, so neither route
///    is an oracle;
/// 2. [`authorise_existing_rule`] passes (`SRC-7` D7: connector, upload and
///    quality rules);
/// 3. [`crate::routes::chart_alerts::may_manage`]: `alert:write`, or their own
///    chart rule (C3 lets a person without the permission own one).
///
/// # Errors
///
/// 401 with no caller; 404 for an unknown rule or one they cannot see; 503 when
/// the rules cannot be read; 403 `alert:write` when it is not theirs to manage.
pub(crate) async fn authorise_rule_test(
    state: &AppState,
    principal: Option<&Principal>,
    id: &str,
) -> Result<(), ApiError> {
    let Some(caller) = principal else {
        return Err(ApiError::unauthorized());
    };
    let Some(rule) = crate::routes::chart_alerts::stored_rule(state, id).await? else {
        return Err(ApiError::NotFound(RULE_NOT_FOUND.to_owned()));
    };
    // D2.1: the list the caller sees decides, so an id of another tenant's
    // connector rule answers exactly as an unknown one.
    if visible_rules(state, Some(caller), vec![rule.clone()])
        .await?
        .is_empty()
    {
        return Err(ApiError::NotFound(RULE_NOT_FOUND.to_owned()));
    }
    // D2.2: the same scope check as an update or a delete (`SRC-7` D7).
    authorise_existing_rule(state, Some(caller), id).await?;
    // D2.3: `alert:write`, or their own chart rule.
    if !crate::routes::chart_alerts::may_manage(Some(caller), Some(&rule)) {
        return Err(ApiError::PermissionDenied("alert:write".to_owned()));
    }
    Ok(())
}

/// `POST /api/alerts/test?id=` — run one rule now, for a signed-in person
/// (`BI-40` D1).
///
/// This is a person's door, not the scheduler's: `POST /api/alerts/run` and its
/// [`check_run_token`] guard (D4) are unchanged, and this route never calls
/// them. The handler decides, as `POST /api/alerts` does (`BI-40` D2), in this
/// order:
///
/// 1. the rule must be in what the caller's list shows ([`visible_rules`] over
///    the one rule), else 404 — the same body an unknown id gets;
/// 2. [`authorise_existing_rule`] must pass (`SRC-7` D7: connector, upload and
///    quality rules);
/// 3. [`crate::routes::chart_alerts::may_manage`]: `alert:write`, or their own
///    chart rule (C3 lets a person without the permission own one).
///
/// The run is [`run_one`] with this id (`BI-40` D4): a real run, not a dry one
/// (`BI-40` D3) — if the condition holds the email or webhook is really sent,
/// the rule's state is settled, a fired result is saved and a recovery
/// resolves. A `pipeline_late` rule answers [`PIPELINE_LATE_TEST`].
///
/// The body is `{ "ran": n, "results": [...] }`, the shape the console's
/// `testRule` reads.
///
/// # `SEC-50` slice 2 T7 (M6/R4)
///
/// The run is gated as the person who asked for it — the same actor every
/// signed-in site of this slice uses ([`crate::routes::gold::gold_export_gate_actor`],
/// the request's `X-Tenant` honoured) — because a person testing a rule reads
/// as themselves, exactly as the assistant's `run_alert_rule` does. It is
/// never the scheduler's catalog tenant: that door is `POST /api/alerts/run`.
///
/// # Refusals (`BI-40` D6)
///
/// 401 only with no session; 403 `alert:write` when the caller may not manage
/// the rule; 404 when the caller cannot see it. Never a 401 for a signed-in
/// person, so the console does not read a refusal as a lost session.
///
/// # Errors
///
/// 400 without an id; 404 as above; 403 as above; 401 with no session; 503 when
/// the rules cannot be read; 500 on a `ClickHouse` failure during the run.
pub async fn test_one(
    State(state): State<AppState>,
    headers: HeaderMap,
    principal: Option<Extension<Principal>>,
    Query(query): Query<TestQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = query.id else {
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    let caller = principal.as_ref().map(|Extension(p)| p);
    authorise_rule_test(&state, caller, &id).await?;
    let actor = crate::routes::gold::gold_export_gate_actor(caller, &headers);
    let mut results = run_one(&state, Some(&id), actor).await?;
    for result in &mut results {
        if result.kind == AlertKind::PipelineLate {
            result.skipped = Some(PIPELINE_LATE_TEST.to_owned());
        }
    }
    Ok(ApiJson(json!({ "ran": results.len(), "results": results })))
}

/// Plan 1f: list every distinct `pipeline_id` referenced by an enabled
/// `pipeline_late` rule with a non-`None`, non-`"*"` `pipeline` field.
/// Used by [`run`] to drive [`evaluate_pipeline_late`]. Filters
/// in-process from `list_rules` to avoid a second `ClickHouse`
/// round-trip and a second schema surface
/// (`pipeline_late_rule_pipelines`) — the underlying query is the same
/// `console.alert_rule` SELECT.
///
/// # Errors
///
/// Returns `Err("database error")` on a `ClickHouse` failure — never an
/// upstream detail, matching AGENTS.md rule 4.
async fn list_late_rule_pipelines(
    ch: &lakehouse_clickhouse::ChClient,
) -> Result<Vec<String>, ApiError> {
    let rules = lakehouse_alerts::list_rules(ch)
        .await
        .map_err(|_| ApiError::Internal("database error".to_owned()))?;
    let mut out: Vec<String> = rules
        .into_iter()
        .filter(|r| {
            r.enabled
                && r.kind == lakehouse_alerts::AlertKind::PipelineLate
                && r.pipeline
                    .as_deref()
                    .is_some_and(|p| !p.is_empty() && p != "*")
        })
        .filter_map(|r| r.pipeline)
        .collect();
    out.sort();
    out.dedup();
    Ok(out)
}

/// Plan 1f: pull the last SUCCESS run (epoch seconds + run id) for every
/// pipeline id in `pipeline_ids` from the Dagster client (one round-trip
/// per id, the same call [`routes::pipelines`] uses for the runs/detail
/// rows). A per-pipeline failure (e.g. Dagster temporarily unreachable)
/// is captured as `None` for that id, matching the same posture
/// `routes::pipelines::last_success_for` takes for a missing run — a
/// missing last-success is then treated by [`late`] as "definitely
/// late, threshold set," which is the honest answer when a pipeline's
/// orchestrator is silent.
///
/// The run id is the dedupe key for the `pipeline_run_event` row written
/// per late episode (reviewer fix #2); without it, two passes for the
/// same "no new success since the threshold" state would re-deliver.
async fn fetch_last_success_per_pipeline(
    dagster: &lakehouse_dagster::DgClient,
    pipeline_ids: &[String],
) -> HashMap<String, Option<LastSuccess>> {
    let mut out = HashMap::new();
    for id in pipeline_ids {
        let job_name = job_name_for_pipeline_id(id);
        let last = dagster
            .list_runs_for_job(&job_name, 30)
            .await
            .ok()
            .and_then(|runs| last_success_for(&runs, &job_name));
        out.insert(id.clone(), last);
    }
    out
}

/// The most recent SUCCESS run's `start_time` epoch seconds and its run
/// id, for the deduplicated `pipeline_run_event` key the late pass uses
/// (reviewer fix #2). `None` when no SUCCESS run is on record yet.
#[derive(Debug, Clone)]
pub(in crate::routes) struct LastSuccess {
    pub(in crate::routes) epoch_seconds: f64,
    pub(in crate::routes) run_id: String,
}

/// Plan 1f: the Dagster job name for `pipeline_id`. Mirrors
/// `routes::pipelines::list_body`'s own `authored__<id>` mapping —
/// `evaluate_pipeline_late` runs against the same `authored__*` jobs
/// the orchestrator launches. `*` is not supported here (see
/// [`list_late_rule_pipelines`]).
fn job_name_for_pipeline_id(pipeline_id: &str) -> String {
    format!("authored__{pipeline_id}")
}

/// Plan 1f: the most recent SUCCESS run for `job_name` in `runs` —
/// `(epoch_seconds, run_id)` of the newest SUCCESS by `start_time`.
/// `None` when no SUCCESS run is present. Mirrors the same pattern as
/// `routes::pipelines::last_success_for` but returns both the epoch
/// seconds (for [`late`]) and the run id (for the dedupe key).
fn last_success_for(runs: &[lakehouse_dagster::DgRun], job_name: &str) -> Option<LastSuccess> {
    runs.iter()
        .filter(|r| r.job_name == job_name && r.status == "SUCCESS")
        .filter_map(|r| r.start_time.map(|t| (t, r.run_id.clone())))
        .fold(None, |best, (t, id)| {
            Some(match best {
                None => LastSuccess {
                    epoch_seconds: t,
                    run_id: id,
                },
                Some(prev) if t > prev.epoch_seconds => LastSuccess {
                    epoch_seconds: t,
                    run_id: id,
                },
                Some(prev) => prev,
            })
        })
}

/// Plan 1f reviewer fix #2: the `pipeline_run_event` row key for a
/// `pipeline_late` episode. The spec says "the last success's run id as
/// the key." When there is no last success (pipeline has never
/// succeeded), this falls back to a per-pipeline sentinel so each
/// pipeline's "no success yet" state still has a unique dedupe key —
/// without it, two pipelines without a success would collide on a
/// `None` run id and the second pipeline's first late episode would
/// silently dedupe away.
fn late_episode_key(pipeline_id: &str, last: Option<LastSuccess>) -> String {
    last.map_or_else(|| format!("{pipeline_id}::no_success"), |s| s.run_id)
}

/// Plan 1f reviewer fix #4 + #2 orchestration: run the late-evaluation
/// pass (one pass per `/api/alerts/run` invocation). Skips entirely when
/// `now_seconds` is `None` — a "late" claim is undecidable without a
/// clock, and the alternative (`Some(0.0)` "late since 1970") is the
/// exact bug the single crate-wide `Option<f64>` clock helper prevents.
/// For each `pipeline_id`, computes the late decision from
/// `LateSource`; when it is `Some(true)`, records the dedupe row
/// (`kind="late"`, `run_id = late_episode_key(...)`) BEFORE delivery
/// (mirrors the run-failed route's pattern at routes/pipelines.rs:1302).
/// `first_seen == false` short-circuits without delivery — that is the
/// sensor-retry / repeat-pass dedupe the spec requires.
///
/// Returned `usize` is the number of pipelines for which delivery was
/// attempted (one per `first_seen` true). Used by the route-level
/// integration test as a "did we deliver at all" signal.
///
/// # Errors
///
/// Returns a 500 [`ApiError::Internal`] when the late source propagates a
/// classified error or a delivery fails; a failed dedupe insert maps
/// through `From<StoreError> for ApiError`.
async fn evaluate_late_pass(
    pg: &PgPool,
    ch: &lakehouse_clickhouse::ChClient,
    http: &lakehouse_notify::WebhookSender,
    email: &EmailSender,
    late_rule_pipelines: Vec<String>,
    last_success: &HashMap<String, Option<LastSuccess>>,
    silence_source: Option<&dyn SilenceSource>,
    now_seconds: Option<f64>,
) -> Result<usize, ApiError> {
    // Reviewer fix #4: when the system clock is unset (an unreachable
    // NTP source on some build hosts), `now_unix_seconds` returns
    // `None`. Without a clock we cannot decide whether any pipeline is
    // currently late, and "late since 1970" would be a worse answer
    // than honest silence. Skip the whole pass with a single
    // `warn!` so an operator can see why their `/api/alerts/run`
    // invocation did not raise anything.
    let Some(now) = now_seconds else {
        tracing::warn!(
            "pipeline_late pass skipped: system clock is unset (SystemTime::now() \
             returned a value before the UNIX epoch)"
        );
        return Ok(0);
    };
    let late_source = ApiLateSource {
        pg,
        last_success: last_success.clone(),
    };
    let mut delivered_pipelines = 0_usize;
    for pipeline_id in &late_rule_pipelines {
        // Reviewer fix #2: dedupe happens BEFORE delivery, the same
        // order `run_failed_event` uses (routes/pipelines.rs:1302).
        // `late_decision` returns the plan 1f `late` helper's result
        // without firing any rule.
        let decision = lakehouse_alerts::late_decision(
            Some(&late_source as &dyn lakehouse_alerts::LateSource),
            pipeline_id,
            Some(now),
        )
        .await
        .map_err(|err| late_error(&err))?;
        if decision != Some(true) {
            continue;
        }
        let dedupe_key = late_episode_key(
            pipeline_id,
            // `get` yields `Option<&Option<_>>`; clone + flatten lifts
            // the inner `Option<LastSuccess>` out in one step.
            last_success.get(pipeline_id).cloned().flatten(),
        );
        // Write the dedupe row FIRST. A `false` return means the same
        // `(run_id, kind)` pair was already inserted by a previous
        // pass for the same episode — short-circuit so this late
        // episode does not re-deliver.
        let first_seen = lakehouse_store::pipelines::record_pipeline_run_event(
            pg,
            &dedupe_key,
            pipeline_id,
            "late",
        )
        .await
        .map_err(ApiError::from)?;
        if !first_seen {
            continue;
        }
        // Delivery — only happens on a fresh insert.
        lakehouse_alerts::evaluate_pipeline_late(
            ch,
            http,
            email,
            pipeline_id,
            Some(&late_source as &dyn lakehouse_alerts::LateSource),
            silence_source,
            Some(now),
        )
        .await
        .map_err(|err| late_error(&err))?;
        delivered_pipelines += 1;
    }
    Ok(delivered_pipelines)
}

// Plan 1f note: `now_seconds` for the late-evaluation pass comes from
// [`crate::routes::pipelines::now_unix_seconds`] — the single crate-wide
// `Option<f64>` clock helper. A `None` from that helper skips the whole
// pass (a broken clock means we cannot make a "late" claim, and the
// alternative — `Some(0.0)` "late since 1970" — is exactly the bug this
// helper exists to prevent).

/// A [`lakehouse_alerts::PipelineLateError`] as a `500`: the `ClickHouse`
/// listing failure is reported (SEC-11); the `LateSource` message is already
/// classified by the source before the alerts crate sees it (its doc comment),
/// so it is ours.
fn late_error(err: &lakehouse_alerts::PipelineLateError) -> ApiError {
    match err {
        lakehouse_alerts::PipelineLateError::ListRules(ch) => upstream_error::ch_error_as(
            &upstream_error::DATABASE,
            ch,
            upstream_error::FailedAs::Internal,
        ),
        lakehouse_alerts::PipelineLateError::Source(message) => ApiError::Internal(message.clone()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    fn service_principal() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "alerts-cron".to_owned(),
            permissions: PermissionSet::default(),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn user_principal() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("*:*"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    #[test]
    fn run_endpoint_rejects_wrong_token_with_401() {
        let err = check_run_token(Some("secret"), None, Some("wrong"), None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
        assert_eq!(err.status(), 401);
    }

    #[test]
    fn run_endpoint_rejects_missing_token_with_401() {
        let err = check_run_token(Some("secret"), None, None, None).unwrap_err();
        assert_eq!(err.to_string(), "unauthorized");
    }

    #[test]
    fn run_endpoint_accepts_matching_header_token() {
        assert!(check_run_token(Some("secret"), Some("secret"), None, None).is_ok());
    }

    #[test]
    fn run_endpoint_accepts_matching_query_token_as_fallback() {
        assert!(check_run_token(Some("secret"), None, Some("secret"), None).is_ok());
    }

    #[test]
    fn run_endpoint_prefers_header_token_over_query_token() {
        // `req.headers.get("x-run-token") || url.searchParams.get("token")`
        // — the header wins when both are present, matching JS `||`.
        assert!(check_run_token(Some("secret"), Some("secret"), Some("wrong"), None).is_ok());
    }

    /// A token, once configured, is still checked even when the caller
    /// happens to also be a service-identity principal — the token path and
    /// the no-token-configured fallback are mutually exclusive branches,
    /// not additive.
    #[test]
    fn configured_token_still_required_even_for_a_service_principal() {
        let service = service_principal();
        let err = check_run_token(Some("secret"), None, Some("wrong"), Some(&service)).unwrap_err();
        assert_eq!(err.status(), 401);
    }

    /// D4 regression: with `ALERTS_RUN_TOKEN` unset, the guard used to pass
    /// unconditionally (see git history / the pre-D4 doc comment) —
    /// including for a caller presenting no token at all and no principal.
    /// It must now fail closed: 503, not a silent pass.
    #[test]
    fn run_endpoint_fails_closed_when_token_unset_and_no_principal() {
        let err = check_run_token(None, None, None, None).unwrap_err();
        assert_eq!(err.status(), 503);
        let err_with_wrong_token = check_run_token(None, Some("anything"), None, None).unwrap_err();
        assert_eq!(err_with_wrong_token.status(), 503);
    }

    /// D4: a human session principal (however highly privileged — even
    /// `*:*`) is still refused when no token is configured. This guard's
    /// whole point is that "some authenticated user" is not enough for a
    /// route that fires real webhooks/emails; only the dedicated
    /// service-identity door is.
    #[test]
    fn run_endpoint_fails_closed_for_a_human_principal_even_with_wildcard_permissions() {
        let user = user_principal();
        let err = check_run_token(None, None, None, Some(&user)).unwrap_err();
        assert_eq!(err.status(), 503);
    }

    /// D4: the new, intended long-term door — a service-identity principal
    /// (e.g. the cron/scheduler's own credential) is let through even with
    /// no shared token configured.
    #[test]
    fn run_endpoint_allows_a_service_identity_principal_when_token_unset() {
        let service = service_principal();
        assert!(check_run_token(None, None, None, Some(&service)).is_ok());
    }

    /// WS5 item C1, WS5 plan review U6: an end-to-end assertion that a
    /// silence suppresses both delivery AND a fresh `alert_instance` row —
    /// not just `lakehouse_store::overview`'s primitive-level boundary
    /// (`overview::tests::a_silenced_rule_produces_no_new_instance_after_its_dedup_window_lapses`
    /// documents that `insert_from_fired_rule` alone has no silence
    /// awareness; this test proves the route layer's decision not to call
    /// it while silenced).
    mod silence_end_to_end {
        use std::collections::HashMap;

        use axum::extract::{Query, State};
        use lakehouse_store::overview::{self, FiredRule};
        use lakehouse_test_support as _;
        use time::OffsetDateTime;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use super::service_principal;
        use crate::config::Config;

        /// One enabled `Alert` rule, `ClickHouse`-row shaped exactly like
        /// `row_to_rule` (`lakehouse-alerts/src/lib.rs`) expects — string
        /// values throughout, matching `FORMAT JSON`'s stringified-integer
        /// convention the rest of this codebase's `ClickHouse` fixtures
        /// already use (see `row_to_rule_maps_freshness_type_column`).
        fn rule_row_json() -> Value {
            json!({
                "id": "al-route-test-1",
                "name": "Route test rule",
                "type": "alert",
                "mart": "route_test_mart",
                "measure": "v",
                "agg": "sum",
                "op": ">",
                "threshold": "10",
                "board": "",
                "channel": "webhook",
                "target": "http://127.0.0.1:1/never-called",
                "enabled": "1",
                "created_at": "",
                "severity": "high",
            })
        }

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

        fn state_for(pool: &sqlx::PgPool, ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            // `SEC-50` slice 2 M11: the rule reads the shared catalog, so the
            // deployment must have a tenant that owns it — the seeded one this
            // test's rule reads `serving.route_test_mart` through. Without it,
            // D6 refuses the rule (several tenants, no `CATALOG_TENANT_ID`) and
            // the test would be about the gate instead of the silence.
            env.insert(
                "CATALOG_TENANT_ID".to_owned(),
                "11111111-1111-4111-8111-000000000001".to_owned(),
            );
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_silenced_rule_is_not_re_delivered_and_produces_no_new_row(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            // `list_rules`' `SELECT ... FROM console.alert_rule` — one
            // enabled Alert rule.
            Mock::given(method("POST"))
                .and(body_string_contains("FROM console.alert_rule"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [rule_row_json()], "rows": 1
                })))
                .mount(&server)
                .await;
            // `current_value`'s `SELECT round(sum(v)) FROM serving.<mart>`
            // — a value far over the rule's threshold of 10, so it fires.
            Mock::given(method("POST"))
                .and(body_string_contains("FROM serving."))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [{ "v": "999" }], "rows": 1
                })))
                .mount(&server)
                .await;
            // `ensure()`'s DDL statements (CREATE DATABASE/TABLE, ADD
            // COLUMN) — a blank 200 for anything else.
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;

            let state = state_for(&pool, &server.uri());
            let pg = state.pg.as_deref().expect("DATABASE_URL was set above");

            // Fixture: the rule already fired once, 20 minutes ago (past
            // the 15-minute dedup window), and is silenced 60 minutes out
            // from now.
            let first = overview::insert_from_fired_rule(
                pg,
                &FiredRule {
                    rule_id: "al-route-test-1",
                    title: "Route test rule",
                    severity: Some("high"),
                    source: "Alert rules",
                    affected: "route_test_mart",
                    detail: "seed",
                },
                OffsetDateTime::now_utc() - time::Duration::minutes(20),
            )
            .await
            .unwrap()
            .unwrap();
            overview::silence_alert(
                pg,
                &first.id,
                OffsetDateTime::now_utc() + time::Duration::minutes(60),
            )
            .await
            .unwrap();

            let response = run(
                State(state),
                HeaderMap::new(),
                Query(RunQuery {
                    id: Some("al-route-test-1".to_owned()),
                    token: None,
                }),
                Some(Extension(service_principal())),
            )
            .await
            .expect("a service-identity principal must pass the run-token guard");

            let body = response.0;
            let result = &body["results"][0];
            assert_eq!(
                result["fired"], true,
                "the rule is still genuinely over threshold"
            );
            assert!(
                result.get("delivered").is_none(),
                "delivery must be suppressed during the silence"
            );

            let row_count: i64 =
                sqlx::query_scalar("SELECT count(*) FROM alert_instance WHERE rule_id = $1")
                    .bind("al-route-test-1")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                row_count, 1,
                "still only the original silenced row — no fresh instance while silenced"
            );
        }
    }

    /// `SEC-10-AC1`: saving a rule whose webhook points at an address
    /// webhooks may not reach is a 400 with the fixed message, through the
    /// real route handlers (`create` and `update`), and nothing is written.
    /// (`SEC-10-AC3`, the permission refusal, is in `tests/route_auth.rs`:
    /// the policy layer answers before either handler runs.)
    mod webhook_targets {
        use std::collections::HashMap;

        use axum::body::Bytes;
        use axum::extract::State;
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use crate::config::Config;

        /// A state over `pool` and a `ClickHouse` at `ch_url`, naming the
        /// catalog's own tenant: the rule these tests save reads
        /// `serving.mart_wisman`, so the tenant gate has to pass for the
        /// handler to reach the webhook check this test is about (`SEC-50`
        /// slice 2 M11 — a fixture change, no assertion weakened).
        fn state_for(pool: &sqlx::PgPool, ch_url: &str) -> AppState {
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
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            env.insert(
                "CATALOG_TENANT_ID".to_owned(),
                "11111111-1111-4111-8111-000000000001".to_owned(),
            );
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        fn body(target: &str, id: Option<&str>) -> Bytes {
            let mut rule = json!({
                "name": "Spike",
                "type": "alert",
                "mart": "mart_wisman",
                "measure": "jumlah",
                "agg": "sum",
                "op": ">",
                "threshold": 1,
                "channel": "webhook",
                "target": target,
            });
            if let Some(id) = id {
                rule["id"] = json!(id);
            }
            Bytes::from(serde_json::to_vec(&rule).expect("serialisable"))
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn saving_an_internal_webhook_is_a_400_with_the_fixed_message(pool: sqlx::PgPool) {
            let ch = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_string(""))
                .mount(&ch)
                .await;
            let state = state_for(&pool, &ch.uri());
            for target in [
                "http://127.0.0.1:8080/x",
                "http://10.1.2.3/x",
                "http://169.254.169.254/latest/meta-data",
                "http://[::1]/x",
            ] {
                for update in [false, true] {
                    let bytes = body(target, update.then_some("al-1"));
                    let result = if update {
                        update_rule(&state, bytes).await
                    } else {
                        create(State(state.clone()), HeaderMap::new(), None, bytes).await
                    };
                    let Err(rejection) = result else {
                        panic!("{target} must be refused");
                    };
                    assert_eq!(rejection.0.status(), 400, "{target}");
                    assert_eq!(
                        rejection.0.to_string(),
                        "webhook address is not allowed",
                        "{target}"
                    );
                }
            }
            let inserts = ch
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|r| {
                    String::from_utf8_lossy(&r.body).contains("INSERT INTO console.alert_rule")
                })
                .count();
            assert_eq!(inserts, 0, "a refused rule must not be written");
        }

        async fn update_rule(state: &AppState, bytes: Bytes) -> ApiResult<ApiJson<Value>> {
            update(State(state.clone()), HeaderMap::new(), None, bytes).await
        }
    }

    /// ADR 0015 — the tick records the schema versions of Silver and Gold
    /// tables beside the quality pass; a single-rule run (`?id=`), which is
    /// someone testing one alert and not the tick, does not.
    mod schema_pass {
        use std::collections::HashMap;
        use std::time::Duration;

        use axum::extract::{Query, State};
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use super::service_principal;
        use crate::config::Config;

        /// A state with no `PostgreSQL` pool (a malformed `DATABASE_URL`,
        /// the idiom `routes::gold`'s tests use) and `ClickHouse` at `ch_url`.
        fn state_for(ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        async fn call_run(state: &AppState, id: Option<&str>) -> Value {
            run(
                State(state.clone()),
                HeaderMap::new(),
                Query(RunQuery {
                    id: id.map(str::to_owned),
                    token: None,
                }),
                Some(Extension(service_principal())),
            )
            .await
            .expect("a service-identity principal must pass the run-token guard")
            .0
        }

        async fn reads_of_system_columns(server: &MockServer) -> usize {
            server
                .received_requests()
                .await
                .expect("recorded")
                .iter()
                .filter(|r| String::from_utf8_lossy(&r.body).contains("FROM system.columns"))
                .count()
        }

        #[tokio::test]
        async fn a_single_rule_run_does_not_start_a_schema_pass() {
            let server = MockServer::start().await;
            // Every statement answers `200` with no rows: no rules, no tables.
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let state = state_for(&server.uri());

            let body = call_run(&state, Some("al-no-such-rule")).await;

            assert_eq!(body["schemaPassStarted"], json!(false));
            assert_eq!(body["qualityPassStarted"], json!(false));
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(reads_of_system_columns(&server).await, 0);
        }

        #[tokio::test]
        async fn the_tick_starts_a_schema_pass_that_reads_the_engine() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&server)
                .await;
            let state = state_for(&server.uri());

            // One pass runs at a time in the whole process, and a pass that
            // another test started (a run-finished report starts one too) may
            // still be running when this tick comes: that answers `false`,
            // so ask again until a pass of ours starts.
            let mut started = false;
            for _ in 0..100 {
                if call_run(&state, None).await["schemaPassStarted"] == json!(true) {
                    started = true;
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert!(started, "a tick must start the schema pass");

            // The pass is in the background: wait for it to reach the engine.
            for _ in 0..200 {
                if reads_of_system_columns(&server).await > 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            panic!("the schema pass the tick started never read the engine's columns");
        }
    }

    /// Plan 1f reviewer fix #2 + #4 — the `pipeline_late` pass inside
    /// `/api/alerts/run`: the clock contract (`None` skips the whole
    /// pass without touching Postgres or `ClickHouse`) and the
    /// per-episode dedupe (a second pass for the same last-success
    /// run id must not re-deliver, via `pipeline_run_event`).
    mod late_pass {
        use lakehouse_clickhouse::ChClient;
        use lakehouse_notify::{EmailSender, SmtpConfig};
        use wiremock::MockServer;
        use wiremock::matchers::method;
        use wiremock::{Mock, ResponseTemplate};

        use super::super::{LastSuccess, evaluate_late_pass};
        use super::*;

        fn no_smtp_email_sender() -> EmailSender {
            EmailSender::new(SmtpConfig {
                host: None,
                port: 587,
                secure: false,
                user: None,
                pass: String::new(),
                from: String::new(),
                ca_bundle: None,
            })
        }

        /// A `ClickHouse` client pointed at `server`, whose only mock
        /// answers `list_rules`' SELECT with zero rows — enough for the
        /// pass to run to completion (with no rules, nothing is
        /// delivered) while still recording every request it receives,
        /// so a test can assert nobody dialed `ClickHouse` at all.
        async fn ch_for(server: &MockServer) -> ChClient {
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [], "rows": 0
                })))
                .mount(server)
                .await;
            ChClient::new(server.uri(), "default".to_owned(), String::new())
        }

        /// Reviewer fix #4: `now_unix_seconds()` returning `None` (no
        /// usable system clock) must skip the ENTIRE pass — no SLA
        /// lookup, no rule listing, no delivery attempt. The wiremock
        /// request log is the proof: an empty log means `ClickHouse`
        /// was never dialed, so nothing downstream of the clock check
        /// ran.
        ///
        /// The fixture is deliberately load-bearing: the last success
        /// sits at a negative epoch so the pipeline WOULD be decided
        /// late if the pass ran under the old `Some(0.0)` fallback
        /// (0.0 − (−3600) > 60). With a realistic "an hour ago" success
        /// the old fallback would evaluate to not-late and this test
        /// would pass either way — proving nothing. A negative epoch is
        /// absurd as real data; it is exactly the value that separates
        /// "skipped" from "evaluated", which is the contract under
        /// test.
        #[sqlx::test(migrations = "../../migrations")]
        async fn late_pass_skips_entirely_when_the_clock_is_unset(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            let ch = ch_for(&server).await;
            let http = crate::webhook_guard::sender(
                &Config::from_map(&HashMap::new()).expect("a valid test Config"),
            );
            let email = no_smtp_email_sender();
            // Threshold 60s, so under the old fallback the run at
            // epoch −3600 is 3600 seconds late.
            lakehouse_store::pipelines::upsert_pipeline_sla(
                &pool,
                "pl-clock-test",
                None,
                Some(60),
                uuid::Uuid::nil(),
            )
            .await
            .unwrap();
            let mut last_success = HashMap::new();
            last_success.insert(
                "pl-clock-test".to_owned(),
                Some(LastSuccess {
                    epoch_seconds: -3600.0,
                    run_id: "run-clock".to_owned(),
                }),
            );

            let delivered = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-clock-test".to_owned()],
                &last_success,
                None,
                None, // the broken-clock case under test
            )
            .await
            .expect("a skipped pass is not an error");

            assert_eq!(delivered, 0, "nothing is delivered without a clock");
            let dialed = server.received_requests().await.unwrap();
            assert!(
                dialed.is_empty(),
                "the pass must not touch ClickHouse when the clock is unset, got {} requests",
                dialed.len()
            );
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM pipeline_run_event")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(events, 0, "no dedupe row is written without a clock");
        }

        /// Reviewer fix #2: the late pass dedupes per episode through
        /// `pipeline_run_event` keyed by the last success's run id. The
        /// first pass for an episode inserts the row and delivers (the
        /// return counts it); a second pass for the SAME episode must
        /// short-circuit on the dedupe row — return 0, and still
        /// exactly one `pipeline_run_event` row.
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_second_late_pass_for_the_same_episode_does_not_re_deliver(pool: sqlx::PgPool) {
            let server = MockServer::start().await;
            let ch = ch_for(&server).await;
            let http = crate::webhook_guard::sender(
                &Config::from_map(&HashMap::new()).expect("a valid test Config"),
            );
            let email = no_smtp_email_sender();
            // The pipeline is late: threshold 60s, last success an hour
            // before `now`.
            lakehouse_store::pipelines::upsert_pipeline_sla(
                &pool,
                "pl-dedupe-test",
                None,
                Some(60),
                uuid::Uuid::nil(),
            )
            .await
            .unwrap();
            let now = 1_800_000_000.0;
            let mut last_success = HashMap::new();
            last_success.insert(
                "pl-dedupe-test".to_owned(),
                Some(LastSuccess {
                    epoch_seconds: now - 3600.0,
                    run_id: "run-abc".to_owned(),
                }),
            );

            let first = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-dedupe-test".to_owned()],
                &last_success,
                None,
                Some(now),
            )
            .await
            .expect("the first pass must succeed");
            assert_eq!(first, 1, "the first pass delivers the episode once");

            let second = evaluate_late_pass(
                &pool,
                &ch,
                &http,
                &email,
                vec!["pl-dedupe-test".to_owned()],
                &last_success,
                None,
                Some(now),
            )
            .await
            .expect("a repeat pass must not error");
            assert_eq!(
                second, 0,
                "the second pass for the same episode is deduped away"
            );

            // Exactly one dedupe row, keyed by the last success's run
            // id — the spec's key choice, pinned here so a drift back
            // to an unspecified key fails loudly.
            let rows: Vec<(String, String, String)> =
                sqlx::query_as("SELECT run_id, pipeline_id, kind FROM pipeline_run_event")
                    .fetch_all(&pool)
                    .await
                    .unwrap();
            assert_eq!(
                rows,
                vec![(
                    "run-abc".to_owned(),
                    "pl-dedupe-test".to_owned(),
                    "late".to_owned()
                )],
                "one row, keyed by the last success's run id"
            );
        }

        /// The dedupe key for a pipeline that has never succeeded is
        /// the per-pipeline sentinel, not an empty string or `None`:
        /// two pipelines without a success must not collide on one key
        /// (the second one's first late episode would silently dedupe
        /// away).
        #[test]
        fn the_no_success_episode_key_is_per_pipeline() {
            assert_eq!(
                super::super::late_episode_key("pl-a", None),
                "pl-a::no_success"
            );
            assert_eq!(
                super::super::late_episode_key(
                    "pl-b",
                    Some(LastSuccess {
                        epoch_seconds: 1.0,
                        run_id: "r1".to_owned(),
                    })
                ),
                "r1"
            );
        }
    }
}

#[cfg(test)]
mod sql_gate_enforcement {
    //! Alerts/digests must be governed like the dashboards they summarize:
    //! same two-harness shape as `support::run_spec_sql_enforcement` (a real
    //! ephemeral Postgres policy row + a wiremock `ClickHouse`).

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_alerts::{GateRefusal, SqlGate};
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::ApiSqlGate;
    use crate::config::Config;

    /// The tenant that owns the shared catalog in these tests — the first
    /// one `0002_seed_identity.sql` creates (`SEC-50` slice 2 M11).
    pub(super) const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";

    /// An [`AppState`] with `pool` as its Postgres and a `ClickHouse` at
    /// `ch_url`, built through `Config` (the production path), naming the
    /// catalog's own tenant so the shared catalog has one (`SEC-50` slice 2
    /// D2/M11) — without it every `serving.*` statement is refused
    /// (`CATALOG_TENANT_ID` names the tenant that owns the shared catalog).
    pub(super) fn state_for(pool: &PgPool, ch_url: &str) -> super::AppState {
        state_for_with(pool, ch_url, Some(CATALOG_TENANT))
    }

    /// The same state, with `CATALOG_TENANT_ID` set only when
    /// `catalog_tenant` names one — a deployment with several tenants and no
    /// catalog tenant is its own case (`SEC-50` D6).
    pub(super) fn state_for_with(
        pool: &PgPool,
        ch_url: &str,
        catalog_tenant: Option<&str>,
    ) -> super::AppState {
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
        env.insert("CH_URL".to_owned(), ch_url.to_owned());
        if let Some(tenant) = catalog_tenant {
            env.insert("CATALOG_TENANT_ID".to_owned(), tenant.to_owned());
        }
        super::AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_digest_or_alert_statement_is_masked_for_the_dashboard_viewer_role(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "alert-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Dashboard Viewer".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Dashboard Viewer"],"table":"serving.mart_x","mask":["email"]}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "amount", "default_kind": "", "default_expression": ""},
                ],
                "rows": 2,
            })))
            .mount(&server)
            .await;
        let state = state_for(&pool, &server.uri());
        let gate = ApiSqlGate {
            // `SEC-50` slice 2 T7: the run route acts as the catalog tenant.
            state: &state,
            actor: crate::routes::query::GateActor::CatalogTenant,
        };

        let governed = gate
            .gate(
                None,
                "SELECT email, sum(amount) AS v FROM serving.mart_x GROUP BY email",
            )
            .await
            .expect("a governed table is rewritten, not refused");

        assert!(
            governed.contains("replaceRegexpOne(toString(`email`)"),
            "the masked column must be rewritten: {governed}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_statement_the_engine_refuses_is_refused_with_a_fixed_message(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        let state = state_for(&pool, &server.uri());
        let gate = ApiSqlGate {
            state: &state,
            actor: crate::routes::query::GateActor::CatalogTenant,
        };

        let err = gate
            .gate(None, "SELECT * FROM url('http://example.com/x.csv', 'CSV')")
            .await
            .expect_err("a table function is refused");

        let GateRefusal::Refused(message) = err else {
            panic!("a table function is a refusal, not a stop");
        };
        assert!(!message.contains("example.com"), "{message}");
        assert!(server.received_requests().await.unwrap().is_empty());
        Ok(())
    }

    // ── `BI-39` T2: an owned rule reads as its owner (D1), and D5 ────────

    /// Seed a mask policy for `role` over `serving.mart_x`, masking `column`.
    async fn mask_for(pool: &PgPool, role: &str, column: &str) {
        governance::create_policy(
            pool,
            &CreatePolicyInput {
                name: format!("bi-39-mask-{role}"),
                kind: "Row filter".to_owned(),
                subjects: role.to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    serde_json::json!({ "roles": [role], "table": "serving.mart_x", "mask": [column] })
                        .to_string(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
    }

    /// A person whose role is named `role` and carries `permissions`.
    async fn user_with_role(pool: &PgPool, role: &str, permissions: &str, status: &str) -> Uuid {
        let role_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, $3, 'test')",
        )
        .bind(role_id)
        .bind(role)
        .bind(permissions)
        .execute(pool)
        .await
        .expect("insert the role");
        let user = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO app_user (id, name, username, email, status) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(user)
        .bind(format!("{role} person"))
        .bind(user.to_string())
        .bind(format!("{user}@test.invalid"))
        .bind(status)
        .execute(pool)
        .await
        .expect("insert the person");
        sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
            .bind(user)
            .bind(role_id)
            .execute(pool)
            .await
            .expect("attach the role");
        user
    }

    /// A `ClickHouse` that lists the columns a mask needs, so the rewrite can
    /// produce its expression.
    async fn engine_with_columns() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "phone", "default_kind": "", "default_expression": ""},
                    {"name": "amount", "default_kind": "", "default_expression": ""},
                ],
                "rows": 3,
            })))
            .mount(&server)
            .await;
        server
    }

    /// `BI-39` D1: the statement of a rule saved by somebody is rewritten with
    /// that person's roles — their mask, not the Dashboard Viewer role's; and a
    /// rule saved before owners existed (`None`) keeps the viewer role's mask
    /// (D4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_owned_rules_statement_is_rewritten_for_its_owner_and_a_legacy_one_for_the_viewer(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // A role of its own: the seeded ones are fixed, and a policy names a
        // role by name.
        let analyst = format!("BI-39 Analyst {}", Uuid::new_v4());
        mask_for(&pool, "Dashboard Viewer", "email").await;
        mask_for(&pool, &analyst, "phone").await;
        let server = engine_with_columns().await;
        let state = state_for(&pool, &server.uri());
        let owner = user_with_role(&pool, &analyst, "dashboard:read, alert:write", "active").await;
        let gate = ApiSqlGate {
            state: &state,
            actor: crate::routes::query::GateActor::CatalogTenant,
        };
        let sql = "SELECT email, phone, sum(amount) AS v FROM serving.mart_x GROUP BY email, phone";

        let owned = gate
            .gate(Some(&owner.to_string()), sql)
            .await
            .expect("the owner's statement is rewritten, not refused");
        assert!(
            owned.contains("replaceRegexpOne(toString(`phone`)"),
            "the owner's own mask must apply: {owned}"
        );
        assert!(
            !owned.contains("replaceRegexpOne(toString(`email`)"),
            "the Dashboard Viewer role's mask must not apply to the owner: {owned}"
        );

        let legacy = gate
            .gate(None, sql)
            .await
            .expect("a rule saved before owners existed is rewritten");
        assert!(
            legacy.contains("replaceRegexpOne(toString(`email`)"),
            "the fixed viewer role's mask still applies to a legacy rule: {legacy}"
        );
        assert!(
            !legacy.contains("replaceRegexpOne(toString(`phone`)"),
            "a legacy rule does not read as the owner: {legacy}"
        );
        Ok(())
    }

    /// `BI-39` D5: a disabled owner stops the rule with the one shared sentence,
    /// rather than being read with a wider or narrower view.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_disabled_owner_stops_the_rule(pool: PgPool) -> sqlx::Result<()> {
        let server = engine_with_columns().await;
        let state = state_for(&pool, &server.uri());
        let role = format!("BI-39 Disabled {}", Uuid::new_v4());
        let owner = user_with_role(&pool, &role, "dashboard:read, alert:write", "inactive").await;
        let gate = ApiSqlGate {
            state: &state,
            actor: crate::routes::query::GateActor::CatalogTenant,
        };

        let err = gate
            .gate(
                Some(&owner.to_string()),
                "SELECT sum(amount) AS v FROM serving.mart_x",
            )
            .await
            .expect_err("a disabled owner stops the rule");

        assert_eq!(
            err,
            GateRefusal::Stop(lakehouse_alerts::OWNER_GONE),
            "the shared sentence, nothing engine-derived"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        Ok(())
    }

    /// `BI-39` D5: an owner who no longer holds `alert:write` stops the rule —
    /// the permission is what let it send to others (C3).
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_owner_without_the_alert_permission_stops_the_rule(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = engine_with_columns().await;
        let state = state_for(&pool, &server.uri());
        let role = format!("BI-39 NoPermission {}", Uuid::new_v4());
        let owner = user_with_role(&pool, &role, "dashboard:read", "active").await;
        let gate = ApiSqlGate {
            state: &state,
            actor: crate::routes::query::GateActor::CatalogTenant,
        };

        let err = gate
            .gate(
                Some(&owner.to_string()),
                "SELECT sum(amount) AS v FROM serving.mart_x",
            )
            .await
            .expect_err("an owner without the permission stops the rule");

        assert_eq!(
            err,
            GateRefusal::Stop(lakehouse_alerts::OWNER_NO_PERMISSION)
        );
        Ok(())
    }
}

#[cfg(test)]
mod alert_run_tenant_scope {
    //! `SEC-50` slice 2 T7: the scheduled run asks the tenant gate before it
    //! reads anything (`SEC-20`), as the deployment's catalog tenant (D3).
    //!
    //! A deployment with several tenants and no `CATALOG_TENANT_ID` has no
    //! tenant whose reach covers the shared catalog, so a mart rule's
    //! statement is refused with D6's sentence — and the run goes on with
    //! the next rule instead of aborting (`lakehouse-alerts`
    //! `lib.rs`, `run_one`). With the setting set, the same rule reads its
    //! value.
    //!
    //! Two rules, one `ClickHouse` mock that records every statement: the
    //! refusal is proven BOTH by the sentence and by the engine never being
    //! asked for `serving.*`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::sql_gate_enforcement::{state_for, state_for_with};
    use super::*;
    use crate::routes::query::JOB_NEEDS_CATALOG_TENANT;

    /// The cron that calls `/api/alerts/run` (the run-token guard lets a
    /// service identity through with no token configured).
    fn cron() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::nil()),
            tenant_ids: Vec::new(),
            display_name: "alerts-cron".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// One enabled mart rule, in the shape `row_to_rule` reads.
    fn rule_row(id: &str, mart: &str) -> Value {
        json!({
            "id": id,
            "name": id,
            "type": "alert",
            "mart": mart,
            "measure": "v",
            "agg": "sum",
            "op": ">",
            "threshold": "10",
            "board": "",
            // No `SMTP_HOST` in the test config: the delivery attempt is
            // refused locally ("email not configured"), never dialled. The
            // value and the threshold decision are what these tests assert.
            "channel": "email",
            "target": "ops@example.com",
            "enabled": "1",
            "created_at": "",
            "severity": "high",
        })
    }

    /// A `ClickHouse` serving both rules and a value far over their
    /// threshold, recording every statement it is sent.
    async fn engine_for(mart_value: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [rule_row("al-tenant-scope-1", "mart_a"),
                         rule_row("al-tenant-scope-2", "mart_b")],
                "rows": 2,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM serving."))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [{ "v": mart_value }], "rows": 1
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    }

    /// The statements the engine was sent that read a mart.
    async fn mart_statements(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .expect("mock records requests")
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|sql| sql.contains("FROM serving."))
            .collect()
    }

    /// `SEC-50` slice 2 T7 (D3/D6): several tenants and no catalog tenant —
    /// every mart rule's check is failed with D6's sentence, the run goes on
    /// to the next rule, and the engine is never asked for a `serving.*`
    /// row.
    #[sqlx::test(migrations = "../../migrations")]
    async fn every_mart_rule_is_refused_with_d6_and_the_next_rule_still_runs(
        pool: sqlx::PgPool,
    ) -> sqlx::Result<()> {
        let server = engine_for("999").await;
        // The catalog's tenant is NOT named, and the seeded deployment holds
        // four tenants: no tenant's reach covers the shared catalog.
        let state = state_for_with(&pool, &server.uri(), None);

        let response = run(
            State(state),
            HeaderMap::new(),
            Query(RunQuery {
                id: None,
                token: None,
            }),
            Some(Extension(cron())),
        )
        .await
        .expect("a service identity passes the run-token guard");

        let results = response.0["results"].as_array().expect("a list").clone();
        assert_eq!(results.len(), 2, "both rules were evaluated: {results:?}");
        for result in &results {
            assert_eq!(
                result["skipped"], JOB_NEEDS_CATALOG_TENANT,
                "the refusal is D6's own sentence: {result}"
            );
            assert_eq!(result["fired"], false, "{result}");
        }
        assert!(
            mart_statements(&server).await.is_empty(),
            "the refusal precedes the engine: no mart may be read"
        );
        Ok(())
    }

    /// `SEC-50` slice 2 T7 (D3): with the catalog's tenant named, the same
    /// rule reads its value from the shared catalog.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_named_catalog_tenant_reads_the_mart(pool: sqlx::PgPool) -> sqlx::Result<()> {
        let server = engine_for("999").await;
        let state = state_for(&pool, &server.uri());

        let response = run(
            State(state),
            HeaderMap::new(),
            Query(RunQuery {
                id: Some("al-tenant-scope-1".to_owned()),
                token: None,
            }),
            Some(Extension(cron())),
        )
        .await
        .expect("a service identity passes the run-token guard");

        let result = &response.0["results"][0];
        assert!(
            result.get("skipped").is_none(),
            "the catalog's tenant reads the shared catalog: {result}"
        );
        assert_eq!(result["value"], 999.0, "{result}");
        assert_eq!(
            mart_statements(&server).await.len(),
            1,
            "exactly the one rule's statement reached the engine"
        );
        Ok(())
    }
}

#[cfg(test)]
mod save_tenant_scope {
    //! `SEC-50` slice 2 D7a (M7): a mart or metric rule is gated as the
    //! person saving it.
    //!
    //! The scheduled run acts as the deployment's catalog tenant (D3), so
    //! without this a member of another tenant holding `alert:write` could
    //! save a rule over `serving.*` and be delivered the catalog's number.
    //! The save now builds the rule's statement with the very builders the
    //! run uses and sends it through the gate as the saving person: a
    //! refusal is `SEC-20`'s 404, and no row is written.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::Bytes;
    use axum::extract::{Extension, State};
    use axum::http::HeaderMap;
    use sqlx::PgPool;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    /// The tenant that owns the shared catalog in this deployment.
    const CATALOG_TENANT: &str = "11111111-1111-4111-8111-000000000001";
    /// Another tenant of the same deployment.
    const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
    /// `SEC-20`'s refusal of the mart both rules read, byte for byte.
    const REFUSAL: &str = "Table not found or not available to your organisation: serving.mart_x";

    /// A state over `pool` and a `ClickHouse` at `ch_url`, naming the
    /// catalog's tenant: the deployment holds several tenants, and the gate
    /// has to be able to say which one owns the shared catalog.
    fn state_for(pool: &PgPool, ch_url: &str) -> AppState {
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
        env.insert("CH_URL".to_owned(), ch_url.to_owned());
        env.insert("CATALOG_TENANT_ID".to_owned(), CATALOG_TENANT.to_owned());
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    /// A person holding `alert:write` and belonging to [`OTHER_TENANT`] only.
    fn other_tenant_engineer() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(21)),
            tenant_ids: vec![OTHER_TENANT.parse().expect("a seeded tenant id")],
            display_name: "Other tenant engineer".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// The engine a metric rule's statement is built from: the metric, its
    /// mart and its columns; every other statement (the rule read, the
    /// `INSERT`) answers `200` with no rows.
    async fn engine() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.bi_metric"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [{
                    "id": "m_demo", "name": "visits", "label": "Visits",
                    "description": "", "source_kind": "mart", "source_id": "mart_x",
                    "formula": "sum(visitors)", "filters_json": "[]",
                    "time_column": "", "grain": "", "format": "number",
                    "created_by": "ui", "updated_at": "",
                }],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.tables"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [], "data": [{ "n": "1" }], "rows": 1
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "meta": [],
                "data": [{ "name": "visitors", "type": "UInt32" }],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;
        server
    }

    /// The inserts the engine was sent, if any.
    async fn alert_rule_inserts(server: &MockServer) -> usize {
        server
            .received_requests()
            .await
            .expect("mock records requests")
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.body).contains("INSERT INTO console.alert_rule"))
            .count()
    }

    fn mart_rule_body() -> Bytes {
        Bytes::from(
            serde_json::to_vec(&json!({
                "name": "Mart spike",
                "type": "alert",
                "mart": "serving.mart_x",
                "measure": "visitors",
                "agg": "sum",
                "op": ">",
                "threshold": 10,
                "channel": "email",
                "target": "ops@example.com",
            }))
            .expect("serialisable"),
        )
    }

    fn metric_rule_body() -> Bytes {
        Bytes::from(
            serde_json::to_vec(&json!({
                "name": "Metric spike",
                "type": "alert",
                "metric": "visits",
                "op": ">",
                "threshold": 10,
                "channel": "email",
                "target": "ops@example.com",
            }))
            .expect("serialisable"),
        )
    }

    /// A person holding `alert:write` and belonging to the catalog's own
    /// tenant: the same rule they may save.
    fn catalog_tenant_engineer() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(22)),
            tenant_ids: vec![CATALOG_TENANT.parse().expect("a seeded tenant id")],
            display_name: "Catalog tenant engineer".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// The other side of the same rule (`SEC-50` D2): the tenant that owns
    /// the shared catalog saves the rule, and the write happens — so the two
    /// refusals above are the gate answering, not a gate that refuses
    /// everything.
    #[sqlx::test(migrations = "../../migrations")]
    async fn saving_a_mart_rule_as_the_catalog_tenants_member_reaches_the_write(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = engine().await;
        let state = state_for(&pool, &server.uri());

        let result = create(
            State(state),
            HeaderMap::new(),
            Some(Extension(catalog_tenant_engineer())),
            mart_rule_body(),
        )
        .await;

        // The row is read back from a `ClickHouse` that answers no rows, so
        // `save_rule`'s own read-back error is the expected one — never the
        // gate's 404, and never before the `INSERT`.
        if let Err(rejection) = &result {
            assert_ne!(rejection.0.status(), axum::http::StatusCode::NOT_FOUND);
            assert_ne!(rejection.0.to_string(), REFUSAL);
        }
        assert_eq!(
            alert_rule_inserts(&server).await,
            1,
            "the catalog's tenant saves the rule: {result:?}"
        );
        Ok(())
    }

    /// `SEC-50` slice 2 D7a (M7): saving a MART rule as a member of another
    /// tenant is `SEC-20`'s 404, and nothing is written.
    #[sqlx::test(migrations = "../../migrations")]
    async fn saving_a_mart_rule_as_another_tenants_member_is_a_404(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = engine().await;
        let state = state_for(&pool, &server.uri());

        let result = create(
            State(state),
            HeaderMap::new(),
            Some(Extension(other_tenant_engineer())),
            mart_rule_body(),
        )
        .await;

        let rejection = result.expect_err("the shared catalog is not this tenant's to read");
        assert_eq!(rejection.0.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(rejection.0.to_string(), REFUSAL);
        assert_eq!(
            alert_rule_inserts(&server).await,
            0,
            "a refused rule must not be written"
        );
        Ok(())
    }

    /// `SEC-50` slice 2 D7a (M7): the same for a METRIC rule, whose
    /// statement the run builds through the metric path — the save builds it
    /// with the same builder.
    #[sqlx::test(migrations = "../../migrations")]
    async fn saving_a_metric_rule_as_another_tenants_member_is_a_404(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = engine().await;
        let state = state_for(&pool, &server.uri());

        let result = create(
            State(state),
            HeaderMap::new(),
            Some(Extension(other_tenant_engineer())),
            metric_rule_body(),
        )
        .await;

        let rejection = result.expect_err("the shared catalog is not this tenant's to read");
        assert_eq!(rejection.0.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(rejection.0.to_string(), REFUSAL);
        assert_eq!(
            alert_rule_inserts(&server).await,
            0,
            "a refused rule must not be written"
        );
        Ok(())
    }
}

#[cfg(test)]
mod rule_scope_authorisation {
    //! `SRC-7` D7/F7: who may save a rule of the connector and upload kinds.
    //! A real ephemeral Postgres (connector tenants) and a wiremock
    //! `ClickHouse` that answers every rule read with one configured row.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::http::StatusCode;
    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

    const OWN_TENANT: &str = "11111111-1111-4111-8111-000000000001";
    const OTHER_TENANT: &str = "11111111-1111-4111-8111-000000000002";
    const OWN_CONNECTOR: &str = "conn-pg-lakehouse";
    const OTHER_CONNECTOR: &str = "conn-s3-warehouse";

    /// A Data Engineer: `alert:write`, member of [`OWN_TENANT`] only.
    fn engineer() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(7)),
            tenant_ids: vec![OWN_TENANT.parse().unwrap()],
            display_name: "Data Engineer".to_owned(),
            permissions: PermissionSet::parse("alert:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// An administrator who sees every tenant (`*:*`).
    fn admin() -> Principal {
        Principal {
            permissions: PermissionSet::parse("*:*"),
            ..engineer()
        }
    }

    /// `existing` is the `(type, connector)` of the one rule the rule store
    /// answers every read with.
    async fn setup(pool: &sqlx::PgPool, existing: (&str, &str)) -> (AppState, MockServer) {
        for (id, tenant) in [(OWN_CONNECTOR, OWN_TENANT), (OTHER_CONNECTOR, OTHER_TENANT)] {
            sqlx::query("UPDATE connector SET tenant_id = $2 WHERE id = $1")
                .bind(id)
                .bind(Uuid::parse_str(tenant).unwrap())
                .execute(pool)
                .await
                .unwrap();
        }
        let ch = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [], "rows": 1,
                "data": [{
                    "id": "al_x", "name": "Rule", "type": existing.0,
                    "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
                    "board": "", "channel": "webhook", "target": "https://hooks.example.com/x",
                    "enabled": "1", "created_at": "", "severity": "", "pipeline": "",
                    "connector": existing.1,
                }]
            })))
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
        let options = pool.connect_options();
        let mut env = HashMap::new();
        env.insert(
            "DATABASE_URL".to_owned(),
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options.get_database().expect("a named test database"),
            ),
        );
        env.insert("CH_URL".to_owned(), ch.uri());
        (
            AppState::new(Config::from_map(&env).expect("a valid test Config")),
            ch,
        )
    }

    fn rule_body(kind: &str, connector: Option<&str>, id: Option<&str>) -> Bytes {
        Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "id": id, "name": "Load failed", "type": kind, "connector": connector,
                "channel": "webhook", "target": "https://93.184.216.34/x",
                "pipeline": "pl-x",
            }))
            .unwrap(),
        )
    }

    async fn create_as(
        state: &AppState,
        principal: Option<Principal>,
        kind: &str,
        connector: Option<&str>,
    ) -> Result<(), ApiError> {
        create(
            State(state.clone()),
            HeaderMap::new(),
            principal.map(Extension),
            rule_body(kind, connector, None),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    async fn update_as(
        state: &AppState,
        principal: Principal,
        kind: &str,
        connector: Option<&str>,
    ) -> Result<(), ApiError> {
        update(
            State(state.clone()),
            HeaderMap::new(),
            Some(Extension(principal)),
            rule_body(kind, connector, Some("al_x")),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_may_save_a_rule_for_a_connector_of_their_own_tenant(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        for kind in [
            "connector_failure",
            "connector_repeated_failure",
            "connector_success",
        ] {
            create_as(&state, Some(engineer()), kind, Some(OWN_CONNECTOR))
                .await
                .unwrap();
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_who_does_not_see_every_tenant_cannot_save_a_rule_for_all_or_for_uploads(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        for (kind, connector) in [
            ("connector_failure", Some("*")),
            ("connector_disabled", Some("*")),
            ("upload_failure", None),
            ("upload_failure", Some(OWN_CONNECTOR)),
        ] {
            let err = create_as(&state, Some(engineer()), kind, connector)
                .await
                .unwrap_err();
            assert_eq!(err.status(), 403, "{kind} {connector:?}");
            assert_eq!(
                err.to_string(),
                format!("permission_denied: {ALL_SCOPE_DENIED}")
            );
        }
    }

    /// A connector of another tenant is answered exactly as an unknown one
    /// (same status, same text but for the id the caller sent): no oracle.
    #[sqlx::test(migrations = "../../migrations")]
    async fn another_tenants_connector_is_refused_like_an_unknown_one(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let other = create_as(
            &state,
            Some(engineer()),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap_err();
        let unknown = create_as(
            &state,
            Some(engineer()),
            "connector_failure",
            Some("conn-ghost"),
        )
        .await
        .unwrap_err();
        assert_eq!(other.status(), StatusCode::NOT_FOUND.as_u16());
        assert_eq!(other.status(), unknown.status());
        assert_eq!(
            other.to_string().replace(OTHER_CONNECTOR, "ID"),
            unknown.to_string().replace("conn-ghost", "ID")
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_administrator_may_save_a_rule_for_all_connectors_and_for_uploads(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", "*")).await;
        create_as(&state, Some(admin()), "connector_failure", Some("*"))
            .await
            .unwrap();
        create_as(&state, Some(admin()), "upload_failure", None)
            .await
            .unwrap();
        create_as(
            &state,
            Some(admin()),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap();
        // Still an existing connector only: an unknown id is the same 404.
        let err = create_as(
            &state,
            Some(admin()),
            "connector_failure",
            Some("conn-ghost"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status(), StatusCode::NOT_FOUND.as_u16());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_new_kinds_fail_closed_with_no_principal_and_other_kinds_are_unchanged(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let err = create_as(&state, None, "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 401);
        // A pipeline kind is not checked: the engineer saves it as before.
        create_as(&state, Some(engineer()), "pipeline_failure", None)
            .await
            .unwrap();
    }

    /// An update may not take over or move a rule the caller could not have
    /// created: not to `*`, not off a `*` rule, not onto another tenant's
    /// connector.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_update_cannot_move_a_rule_the_caller_could_not_have_created(pool: sqlx::PgPool) {
        // The stored rule is for all connectors.
        let (state, _ch) = setup(&pool, ("connector_failure", "*")).await;
        let err = update_as(&state, engineer(), "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403, "taking over an all-connectors rule");
        // The stored rule is for another tenant's connector.
        let (state, _ch) = setup(&pool, ("connector_failure", OTHER_CONNECTOR)).await;
        let err = update_as(&state, engineer(), "connector_failure", Some(OWN_CONNECTOR))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 404, "taking over another tenant's rule");
        // The stored rule is the caller's own; moving it to `*` or to another
        // tenant's connector is refused, moving it within their tenant is not.
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let err = update_as(&state, engineer(), "connector_failure", Some("*"))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403);
        let err = update_as(
            &state,
            engineer(),
            "connector_failure",
            Some(OTHER_CONNECTOR),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status(), 404);
        update_as(
            &state,
            engineer(),
            "connector_repeated_failure",
            Some(OWN_CONNECTOR),
        )
        .await
        .unwrap();
        // Turning a connector rule into a pipeline kind does not escape the
        // check on what it was.
        let (state, _ch) = setup(&pool, ("upload_failure", "*")).await;
        let err = update_as(&state, engineer(), "pipeline_failure", None)
            .await
            .unwrap_err();
        assert_eq!(err.status(), 403);
        // An administrator may do all of it.
        update_as(&state, admin(), "connector_failure", Some("*"))
            .await
            .unwrap();
    }

    async fn listed_as(state: &AppState, principal: Option<Principal>) -> Result<Value, ApiError> {
        list(State(state.clone()), principal.map(Extension))
            .await
            .map(|json| json.0)
            .map_err(|rejection| rejection.0)
    }

    async fn deleted_as(state: &AppState, principal: Option<Principal>) -> Result<(), ApiError> {
        delete(
            State(state.clone()),
            principal.map(Extension),
            Query(DeleteQuery {
                id: Some("al_x".to_owned()),
            }),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    /// `SRC-7` review BLOCKER 1: the list shows a non-administrator only the
    /// connector rules of their tenants.
    #[sqlx::test(migrations = "../../migrations")]
    async fn list_shows_a_caller_only_the_connector_rules_of_their_tenants(pool: sqlx::PgPool) {
        // The rule store answers every read with this one rule.
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        assert_eq!(body["rules"][0]["connector"], OWN_CONNECTOR);

        let (state, _ch) = setup(&pool, ("connector_failure", OTHER_CONNECTOR)).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"], serde_json::json!([]));

        // A connector that no longer exists is hidden from a non-admin.
        let (state, _ch) = setup(&pool, ("connector_failure", "conn-gone")).await;
        let body = listed_as(&state, Some(engineer())).await.unwrap();
        assert_eq!(body["rules"], serde_json::json!([]));

        // `*` and upload rules are an unrestricted caller's alone.
        for existing in [("connector_failure", "*"), ("upload_failure", "*")] {
            let (state, _ch) = setup(&pool, existing).await;
            let body = listed_as(&state, Some(engineer())).await.unwrap();
            assert_eq!(body["rules"], serde_json::json!([]));
            let body = listed_as(&state, Some(admin())).await.unwrap();
            assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        }
    }

    /// Other kinds are returned as before, to anyone and with no principal;
    /// the new kinds with no principal are a 401.
    #[sqlx::test(migrations = "../../migrations")]
    async fn list_returns_other_kinds_untouched_and_refuses_new_kinds_without_a_principal(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("pipeline_failure", "")).await;
        for principal in [None, Some(engineer())] {
            let body = listed_as(&state, principal).await.unwrap();
            assert_eq!(body["rules"].as_array().unwrap().len(), 1);
        }
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        assert_eq!(listed_as(&state, None).await.unwrap_err().status(), 401);
    }

    /// `SRC-7` review BLOCKER 2: a delete needs the check an update applies to
    /// the stored rule, with the same answer for another tenant's connector
    /// as for an unknown one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn delete_applies_the_scope_check_to_the_stored_rule(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("connector_failure", OWN_CONNECTOR)).await;
        deleted_as(&state, Some(engineer())).await.unwrap();

        let (state, _ch) = setup(&pool, ("connector_failure", OTHER_CONNECTOR)).await;
        let other = deleted_as(&state, Some(engineer())).await.unwrap_err();
        let (state, _ch) = setup(&pool, ("connector_failure", "conn-gone")).await;
        let unknown = deleted_as(&state, Some(engineer())).await.unwrap_err();
        assert_eq!(other.status(), 404);
        assert_eq!(unknown.status(), 404);
        assert_eq!(
            other.to_string().replace(OTHER_CONNECTOR, "X"),
            unknown.to_string().replace("conn-gone", "X")
        );

        for existing in [("connector_failure", "*"), ("upload_failure", "*")] {
            let (state, _ch) = setup(&pool, existing).await;
            assert_eq!(
                deleted_as(&state, Some(engineer()))
                    .await
                    .unwrap_err()
                    .status(),
                403
            );
            deleted_as(&state, Some(admin())).await.unwrap();
        }
        // Other kinds delete as before.
        let (state, _ch) = setup(&pool, ("pipeline_failure", "")).await;
        deleted_as(&state, Some(engineer())).await.unwrap();
    }

    // ── DATA-14: quality incident rules are for an administrator ──────────

    fn quality_body(asset: Option<&str>, id: Option<&str>) -> Bytes {
        Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "id": id, "name": "Quality incident", "type": "quality_incident",
                "asset": asset, "channel": "webhook", "target": "https://93.184.216.34/x",
            }))
            .unwrap(),
        )
    }

    async fn create_quality_as(
        state: &AppState,
        principal: Option<Principal>,
        asset: Option<&str>,
    ) -> Result<(), ApiError> {
        create(
            State(state.clone()),
            HeaderMap::new(),
            principal.map(Extension),
            quality_body(asset, None),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
    }

    /// The message names tables and rules of any tenant, so a user who does not
    /// see every tenant cannot save, move or delete such a rule, whatever the
    /// table; the refusal is the same text, and fails closed with no principal.
    #[sqlx::test(migrations = "../../migrations")]
    async fn only_an_administrator_saves_a_quality_incident_rule(pool: sqlx::PgPool) {
        let (state, _ch) = setup(&pool, ("quality_incident", "")).await;
        for asset in [Some("*"), Some("silver.orders"), None] {
            let err = create_quality_as(&state, Some(engineer()), asset)
                .await
                .unwrap_err();
            assert_eq!(err.status(), 403, "{asset:?}");
            assert_eq!(
                err.to_string(),
                format!("permission_denied: {QUALITY_SCOPE_DENIED}")
            );
        }
        let err = create_quality_as(&state, None, Some("*"))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 401);

        create_quality_as(&state, Some(admin()), Some("*"))
            .await
            .unwrap();
        create_quality_as(&state, Some(admin()), Some("silver.orders"))
            .await
            .unwrap();
        // The administrator's own validation still applies: a bad table is a 400.
        let err = create_quality_as(&state, Some(admin()), Some("orders"))
            .await
            .unwrap_err();
        assert_eq!(err.status(), 400);

        // An update or a delete of a stored quality rule is refused the same way.
        let err = update(
            State(state.clone()),
            HeaderMap::new(),
            Some(Extension(engineer())),
            quality_body(Some("silver.orders"), Some("al_x")),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
        .unwrap_err();
        assert_eq!(err.status(), 403);
        let err = delete(
            State(state.clone()),
            Some(Extension(engineer())),
            Query(DeleteQuery {
                id: Some("al_x".to_owned()),
            }),
        )
        .await
        .map(|_| ())
        .map_err(|rejection| rejection.0)
        .unwrap_err();
        assert_eq!(err.status(), 403);
    }

    /// The list hides quality incident rules (and their webhook targets) from
    /// everyone but an administrator.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_list_hides_quality_incident_rules_from_a_user_who_does_not_see_every_tenant(
        pool: sqlx::PgPool,
    ) {
        let (state, _ch) = setup(&pool, ("quality_incident", "")).await;
        let rule = |id: &str, kind: AlertKind| AlertRule {
            id: id.to_owned(),
            name: id.to_owned(),
            kind,
            mart: None,
            measure: None,
            agg: "sum".to_owned(),
            op: lakehouse_alerts::AlertOp::Gt,
            threshold: 0.0,
            board: None,
            channel: lakehouse_alerts::AlertChannel::Webhook,
            target: "https://hooks.example.com/secret".to_owned(),
            enabled: true,
            created_at: None,
            severity: None,
            pipeline: None,
            metric: None,
            connector: None,
            asset: (kind == AlertKind::QualityIncident).then(|| "*".to_owned()),
            chart: None,
            series: None,
            filters: Vec::new(),
            condition: None,
            owner_id: None,
        };
        let rules = vec![
            rule("quality", AlertKind::QualityIncident),
            rule("plain", AlertKind::Alert),
        ];
        let ids = |list: Vec<AlertRule>| list.into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(
            ids(visible_rules(&state, Some(&engineer()), rules.clone())
                .await
                .unwrap()),
            vec!["plain"]
        );
        assert_eq!(
            ids(visible_rules(&state, Some(&admin()), rules.clone())
                .await
                .unwrap()),
            vec!["quality", "plain"]
        );
        let err = visible_rules(&state, None, rules).await.unwrap_err();
        assert_eq!(err.status(), 401);
    }
}

#[cfg(test)]
mod run_events_configured {
    //! `SRC-7` D9 (review SHOULD-FIX 3): `GET /api/alerts/status` says whether
    //! run reports can arrive.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    async fn reported(token: Option<&str>) -> Value {
        let mut env = HashMap::new();
        if let Some(token) = token {
            env.insert("PIPELINE_RUN_TOKEN".to_owned(), token.to_owned());
        }
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        serde_json::to_value(status(State(state)).await.expect("status").0).unwrap()
    }

    #[tokio::test]
    async fn the_flag_is_true_with_a_token_and_false_without_or_with_an_empty_one() {
        assert_eq!(
            reported(Some("a-token")).await,
            json!({ "runEventsConfigured": true })
        );
        assert_eq!(
            reported(None).await,
            json!({ "runEventsConfigured": false })
        );
        assert_eq!(
            reported(Some("")).await,
            json!({ "runEventsConfigured": false })
        );
    }
}

/// `DEC-16` (QA `LH-SRC-7-04`, `SRC-42`): a rule's target is returned only to
/// a caller who may manage the rule. Plain `#[test]`: no database.
#[cfg(test)]
mod hidden_targets {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_alerts::{AlertChannel, AlertKind, AlertOp, AlertRule};
    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use serde_json::json;
    use uuid::Uuid;

    use super::rule_json_for;

    const ADDRESS: &str = "https://hooks.example.com/secret-token";

    fn principal(permissions: &str, id: u128) -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(id)),
            tenant_ids: Vec::new(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse(permissions),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn rule(chart: Option<&str>, owner: Option<&str>) -> AlertRule {
        AlertRule {
            id: "al_1".to_owned(),
            name: "orders".to_owned(),
            kind: AlertKind::Alert,
            mart: Some("mart_orders".to_owned()),
            measure: Some("orders".to_owned()),
            agg: "sum".to_owned(),
            op: AlertOp::Gt,
            threshold: 10.0,
            board: None,
            channel: AlertChannel::Webhook,
            target: ADDRESS.to_owned(),
            enabled: true,
            created_at: None,
            severity: None,
            pipeline: None,
            metric: None,
            connector: None,
            asset: None,
            chart: chart.map(str::to_owned),
            series: None,
            filters: Vec::new(),
            condition: None,
            owner_id: owner.map(str::to_owned),
        }
    }

    #[test]
    fn a_caller_with_alert_read_only_gets_the_rule_without_its_target() {
        let value = rule_json_for(Some(&principal("alert:read", 1)), &rule(None, None)).unwrap();
        assert!(value.get("target").is_none(), "{value}");
        assert_eq!(value["targetHidden"], json!(true));
        assert_eq!(value["channel"], json!("webhook"));
        assert!(!value.to_string().contains(ADDRESS), "{value}");
    }

    #[test]
    fn a_caller_with_alert_write_gets_the_target() {
        let value = rule_json_for(
            Some(&principal("alert:read, alert:write", 1)),
            &rule(None, None),
        )
        .unwrap();
        assert_eq!(value["target"], json!(ADDRESS));
        assert!(value.get("targetHidden").is_none(), "{value}");
    }

    #[test]
    fn the_owner_of_a_chart_alert_gets_their_own_target_without_alert_write() {
        let owner = principal("alert:read", 7);
        let owner_id = owner.id.uuid().to_string();
        let value = rule_json_for(Some(&owner), &rule(Some("chart-1"), Some(&owner_id))).unwrap();
        assert_eq!(value["target"], json!(ADDRESS));
        assert!(value.get("targetHidden").is_none(), "{value}");

        // Another person's chart rule, and a mart rule of their own, stay
        // hidden: only a chart alert they own carries their own address.
        let someone_else = Uuid::from_u128(9).to_string();
        let other =
            rule_json_for(Some(&owner), &rule(Some("chart-1"), Some(&someone_else))).unwrap();
        assert!(other.get("target").is_none(), "{other}");
        let mart = rule_json_for(Some(&owner), &rule(None, Some(&owner_id))).unwrap();
        assert!(mart.get("target").is_none(), "{mart}");
    }

    #[test]
    fn a_caller_with_no_principal_gets_no_target() {
        let value = rule_json_for(None, &rule(None, None)).unwrap();
        assert!(value.get("target").is_none(), "{value}");
        assert_eq!(value["targetHidden"], json!(true));
    }
}
