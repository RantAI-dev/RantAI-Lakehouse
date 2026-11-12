//! Per-tool implementations, grouped by domain, and the dispatch that
//! calls one by name.
//!
//! - [`data`] — `run_sql`, `list_datasets`, `describe_dataset`,
//!   `get_lineage`, `get_quality`, `describe_mart`.
//! - [`ask`] — `ask_user`, the question with options the console shows as
//!   buttons.
//! - [`propose`] — `propose_sql`, a corrected statement checked for the
//!   caller and shown to the person, never run.
//! - [`look`] — `format_chart`, `set_dashboard_theme` (`BI-17` part A) and
//!   `set_cache_rule` (`BI-4` part B).
//! - [`dashboards`] — `create_chart`, `update_chart`, `delete_chart`,
//!   `create_board`, `list_boards`, `list_charts`, `list_sql_sources`,
//!   `suggest_dashboard`.
//! - [`export`] — `export_dashboard` and `export_chart` (`AI-10`, with `BI-1`): a
//!   download link, made when the person opens it.
//! - [`reports`] — the scheduled-report tools (`AI-10`, with `RPT-1`).
//! - [`tabs`] — the dashboard tab tools (`BI-18` part C, with `AI-9`).
//! - [`calc_fields`] — the calculated-field tools (`BI-8`, with `AI-4`).
//! - [`embed`] — the signed-embed tools (`BI-26` slice 2, with `AI-11-AC3`):
//!   enable, filter modes, withdraw, and the snippet.//! - [`pipelines`] — `trigger_lakehouse_build`, `get_build_status`, plus
//!   the Tier 1 pipeline-operations tools (T1.3).
//! - [`alerts`] — Tier 1 alert-rule tools (T1.1).
//! - [`connectors`] — Tier 1 connector tools (T1.2).
//! - [`queries`] — Tier 1 saved-query tools (T1.4).
//! - [`governance`] — Tier 2 governance reads + draft tools (T2.1, T2.5).
//! - [`ops`] — Tier 2 workload tools (T2.3).
//! - [`gold`] — Tier 2 Gold export tools (T2.4). `run_bronze_maintenance`
//!   (T2.2) lives in [`pipelines`] alongside the other `DgClient::launch_run`
//!   caller (`trigger_build`), rather than a one-tool `maintenance` module.

mod alerts;
mod ask;
mod boards;
mod calc_fields;
mod comments;
mod connectors;
mod dashboards;
mod embed;
mod export;
mod maps;
// `pub(in crate::routes)` (not merely `mod`) so `routes::lineage` — a
// SIBLING module of `routes::ai`, not a descendant — can reuse
// `data::CATALOG_UNION`, the SAME bronze-catalog query `list_datasets`
// already runs, rather than re-deriving it.
pub(in crate::routes) mod data;
mod gold;
mod governance;
mod ingest;
mod lakehouse;
mod limits;
mod look;
#[cfg(test)]
pub(super) use look::FORMAT_FIELDS;
mod metrics;
mod ops;
mod pipelines;
mod propose;
mod queries;
mod reports;
pub(in crate::routes) use reports::{approval_args, report_audience};
mod tabs;

use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_bi::access::{Level, Viewer};
use serde_json::{Map, Value, json};

use super::registry;
use crate::error::ApiResult;
use crate::state::AppState;

/// Turns an axum [`Response`] into the [`Value`] a copilot tool returns,
/// by reading its (always-`ApiJson`, hence always-JSON) body back out.
///
/// This is how the Tier 1 tools (T1.1-T1.4) REUSE the actual console route
/// handlers — `routes::pipelines::{list,runs,trigger,pause,resume,
/// cancel_run,retry_run}` already return a plain [`Response`], built with
/// [`crate::json::ApiJson`] — rather than re-implementing their branching
/// (authored-vs-Dagster dispatch, 404/409/503 mapping, ...) a second time
/// for the copilot. Calling the handler directly means a guard or a bug fix
/// applied to the console route is automatically applied to the copilot
/// tool too, with no risk of the two drifting apart.
///
/// A malformed/non-JSON body (never true for an `ApiJson` response in
/// practice) falls back to `{}` rather than panicking — `unwrap`/`expect`
/// are denied outside test modules, and a copilot tool must always return
/// *some* JSON `Value`, not fail the whole chat turn over its own
/// response-parsing.
pub(super) async fn response_to_value(resp: Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
}

/// Same as [`response_to_value`], for the handlers (`routes::connectors::*`,
/// `routes::query::run`, `routes::alerts::{list,create,update,delete}`)
/// that return [`ApiResult<T>`] rather than a bare [`Response`] — `T`'s own
/// [`IntoResponse`] impl (`ApiJson<T>`, or a tuple like
/// `(StatusCode, ApiJson<T>)`) and [`crate::error::ApiRejection`]'s both
/// produce a JSON body, so routing either arm through
/// [`response_to_value`] gives the same "call the real handler, read its
/// JSON back" reuse as pipelines' bare-`Response` handlers.
pub(super) async fn api_result_to_value<T: IntoResponse>(result: ApiResult<T>) -> Value {
    match result {
        Ok(ok) => response_to_value(ok.into_response()).await,
        Err(err) => response_to_value(err.into_response()).await,
    }
}

/// Dispatch one tool call by name, matching `runTool` in `ai-tools.ts`.
/// Unknown tool names return `{"error": "unknown tool: <name>"}` rather
/// than failing the request — the LLM sees the error and can recover.
///
/// The unknown-name check goes through [`registry::find`] first — the
/// single source of truth for which names are real tools — so this
/// `match`'s arms and the registry can never quietly drift apart: a name
/// [`registry::find`] recognises but this `match` has no arm for hits the
/// `unreachable!` below instead of silently falling through to "tool tak
/// dikenal", which would be a bug worth crashing a test over, not masking.
///
/// The match is the registry-to-domain lookup and nothing more: 66 arms
/// naming a tool, plus one guard arm (`n if calc_fields::handles(n)`),
/// plus a catch-all, over 16 per-domain modules. All but
/// `create_chart | update_chart` are a plain delegation (rustfmt wraps some
/// onto three lines, as for `set_pipeline_target_lag`); that one reads the
/// report time zone first, because a chart's SQL is built in it (`BI-9`).
#[allow(
    clippy::too_many_lines,
    reason = "one lookup table over 16 domain modules, 66 of its 67 arms a \
              delegation; the split by domain already happened, so \
              splitting the table itself would only scatter that lookup"
)]
pub(in crate::routes) async fn run_tool(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    if registry::find(name).is_none() {
        return json!({ "error": format!("unknown tool: {name}") });
    }
    // The switch hides `ask_user` from the chat's tool list, but a model can
    // name a tool it was not given and the gate does not know the switch, so
    // the refusal sits here, where `/api/ai/tool` and the chat both run tools.
    // PR review fix (SHOULD-FIX): with `AI_ASK_BACK=false` the call still ran.
    if name == registry::ASK_USER && !state.config.ai_ask_back {
        return json!({ "error": "asking the user is switched off" });
    }
    // Same shape as the refusal above: `propose_sql` is hidden from the list
    // when `AI_SQL_HELP` is off, and this refuses a call the model makes anyway.
    if name == registry::PROPOSE_SQL && !state.config.ai_sql_help {
        return json!({ "error": "SQL help is switched off" });
    }
    let ch = &state.clickhouse;
    match name {
        "run_sql" => data::run_sql(state, principal, args).await,
        "ask_user" => ask::ask_user(args),
        "propose_sql" => propose::propose_sql(state, principal, args).await,
        "list_datasets" => data::list_datasets(state, principal, args).await,
        "lakehouse_overview" => data::lakehouse_overview(state, principal).await,
        "describe_dataset" => data::describe_dataset(state, principal, args).await,
        "get_lineage" => data::get_lineage(state, principal, args).await,
        "get_quality" => data::get_quality(ch).await,
        "trigger_lakehouse_build" => pipelines::trigger_build(state, principal).await,
        "get_build_status" => pipelines::get_build_status(&state.dagster).await,
        "describe_mart" => data::describe_mart(state, principal, args).await,
        "create_chart" | "update_chart" => {
            let scope = match require_chart_call_allowed(state, principal, name, args).await {
                Ok(scope) => scope,
                Err(error) => return error,
            };
            let viewer = &scope.viewer;
            // BI-9: a chart's SQL is built with the report time zone (a deployment setting).
            let Ok(time) = crate::routes::settings::time_context(state).await else {
                // The cause was logged under a reference by `time_context`.
                return json!({ "error": "The reporting settings could not be read." });
            };
            if name == "create_chart" {
                dashboards::create_chart(ch, viewer, principal, args, None, &time).await
            } else {
                dashboards::update_chart(ch, viewer, principal, args, &time).await
            }
        }
        "create_board" => dashboards::create_board(ch, principal, args).await,
        "list_boards" => match viewer_for(state, principal).await {
            Ok(viewer) => dashboards::list_boards(ch, &viewer).await,
            Err(error) => error,
        },
        "suggest_dashboard" => dashboards::suggest_dashboard(ch).await,
        "list_charts" => match viewer_for(state, principal).await {
            Ok(viewer) => dashboards::list_charts(ch, &viewer).await,
            Err(error) => error,
        },
        "list_sql_sources" | "suggest_join" => match viewer_for(state, principal).await {
            Ok(viewer) => dashboards::chart_sources(state, &viewer, principal, name, args).await,
            Err(error) => error,
        },
        "delete_chart" => match scope_for_write(state, principal).await {
            Ok(scope) => {
                // BI-19 B2 review B2-1: `edit` on the chart's dashboard first.
                if let Err(error) =
                    require_edit_on_chart(state, &scope, &arg_str(args, "id"), None).await
                {
                    return error;
                }
                dashboards::delete_chart(ch, &scope.viewer, principal, args).await
            }
            Err(error) => error,
        },
        n if limits::handles(n) => limits::run(state, principal, name, args).await,
        n if look::handles(n) => look::run(state, principal, name, args).await,
        n if export::handles(n) => export::run(state, principal, name, args).await,
        n if reports::handles(n) => reports::run(state, principal, name, args).await,
        "rename_board"
        | "delete_board"
        | "arrange_board"
        | "move_chart"
        | "set_board_view_filters"
        | "save_board_filters" => boards::run(state, principal, name, args).await,
        n if calc_fields::handles(n) => calc_fields::run(state, principal, name, args).await,
        "list_alert_rules" => alerts::list_alert_rules(state, principal).await,
        "create_alert_rule" => alerts::create_alert_rule(state, principal, args).await,
        "update_alert_rule" => alerts::update_alert_rule(state, principal, args).await,
        "delete_alert_rule" => alerts::delete_alert_rule(state, principal, args).await,
        "run_alert_rule" => alerts::run_alert_rule(state, principal, args).await,
        n if comments::handles(n) => {
            comments::list_dashboard_comments(state, principal, args).await
        }
        n if embed::handles(n) => embed::run(state, principal, name, args).await,
        "list_connectors" => connectors::list_connectors(state, principal).await,
        "create_connector" => connectors::create_connector(state, principal, args).await,
        "test_connector" => connectors::test_connector(state, principal, args).await,
        "delete_connector" => connectors::delete_connector(state, principal, args).await,
        "list_pipelines" => pipelines::list_pipelines(state, principal).await,
        "list_pipeline_runs" => pipelines::list_pipeline_runs(state, principal, args).await,
        "trigger_pipeline" => pipelines::trigger_pipeline(state, principal, args).await,
        "retry_pipeline_run" => pipelines::retry_pipeline_run(state, principal, args).await,
        "pause_pipeline" => pipelines::pause_pipeline(state, principal, args).await,
        "resume_pipeline" => pipelines::resume_pipeline(state, principal, args).await,
        "cancel_pipeline_run" => pipelines::cancel_pipeline_run(state, principal, args).await,
        "save_query" => queries::save_query(state, principal, args).await,
        "list_saved_queries" => queries::list_saved_queries(state, principal).await,
        "run_saved_query" => queries::run_saved_query(state, principal, args).await,
        "get_audit_history" => governance::get_audit_history(state).await,
        "list_classification_rules" => governance::list_classification_rules(state).await,
        "list_quality_rules" => governance::list_quality_rules(state).await,
        "get_cdc_health" => governance::get_cdc_health(state).await,
        "get_maintenance_metrics" => governance::get_maintenance_metrics(state).await,
        "run_bronze_maintenance" => pipelines::run_bronze_maintenance(&state.dagster).await,
        "list_workloads" => ops::list_workloads(state).await,
        "kill_query" => ops::kill_query(state, args).await,
        "export_gold_mart" => gold::export_gold_mart(state, principal, args).await,
        "get_gold_export" => gold::get_gold_export(state, args).await,
        "draft_policy" => governance::draft_policy(state, args).await,
        "draft_classification_rule" => governance::draft_classification_rule(state, args).await,
        "draft_quality_rule" => governance::draft_quality_rule(state, principal, args).await,
        "get_ingest_spec" => ingest::get_ingest_spec(state, args).await,
        "set_ingest_spec" => ingest::set_ingest_spec(state, args).await,
        "discover_source" => ingest::discover_source(state, args).await,
        "run_ingest" => ingest::run_ingest(state, args).await,
        "list_ingest_runs" => ingest::list_ingest_runs(state, args).await,
        "rotate_connector_credential" => {
            ingest::rotate_connector_credential(state, principal, args).await
        }
        "create_pipeline" => pipelines::create_pipeline(state, principal, args).await,
        "get_pipeline" => pipelines::get_pipeline(state, args).await,
        "mark_pipeline_ready" => pipelines::mark_pipeline_ready(state, args).await,
        "set_pipeline_target_lag" => {
            pipelines::set_pipeline_target_lag(state, principal, args).await
        }
        "list_iceberg_tables" => lakehouse::list_iceberg_tables(state, principal, args).await,
        "describe_iceberg_table" => lakehouse::describe_iceberg_table(state, principal, args).await,
        "get_table_maintenance" => lakehouse::get_table_maintenance(state, args).await,
        "set_table_maintenance" => lakehouse::set_table_maintenance(state, args).await,
        "get_capacity" => lakehouse::get_capacity(state).await,
        other => {
            unreachable!(
                "registry::find recognised {other:?} but run_tool has no dispatch arm for it"
            )
        }
    }
}

/// The level on the dashboard a call acts on, for the gate (`BI-37` D10):
/// worked out on the server before `gate::decide_with`/`decide_by_name`, never
/// taken from the arguments the model sent. `None` for a tool outside
/// [`registry::DASHBOARD_SCOPED`], for a call with nobody attached, and for a
/// dashboard the caller cannot see or the store cannot read — the gate then
/// falls back to the plain permission (fail closed).
pub(in crate::routes) async fn dashboard_reach(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Option<Level> {
    let reach = registry::dashboard_scoped(name)?;
    let principal = principal?;
    match reach {
        registry::Reach::Board(argument) => {
            crate::routes::dashboard_access::level_on_board(
                state,
                principal,
                &arg_str(args, argument),
            )
            .await
        }
        registry::Reach::Chart(argument) => {
            crate::routes::dashboard_access::level_on_chart(
                state,
                principal,
                &arg_str(args, argument),
            )
            .await
        }
        // `set_cache_rule` names one of the two; the empty one is not filled.
        registry::Reach::BoardOrChart => {
            let board = arg_str(args, "board");
            if board.trim().is_empty() {
                crate::routes::dashboard_access::level_on_chart(
                    state,
                    principal,
                    &arg_str(args, "chart"),
                )
                .await
            } else {
                crate::routes::dashboard_access::level_on_board(state, principal, &board).await
            }
        }
        registry::Reach::Anywhere => dashboard_anywhere(state, Some(principal)).await,
    }
}

/// The highest level the caller holds on any dashboard (`BI-37` D10): what the
/// listing tools may be offered for and what an `Anywhere` tool is judged by.
/// `None` with nobody attached, or when the rules cannot be read.
pub(in crate::routes) async fn dashboard_anywhere(
    state: &AppState,
    principal: Option<&Principal>,
) -> Option<Level> {
    crate::routes::dashboard_access::highest_level(state, principal?).await
}

/// Who a dashboard tool reads as: the caller's viewer after their rules, the
/// restrictions and their starting point (`BI-19` B1 review R1; before this the
/// tools used the baseline and showed a restricted dashboard to any
/// `dashboard:read` holder). With nobody attached the viewer is anonymous (no
/// personal space, nothing in the trash). A refusal or a store failure is a
/// tool error with no fallback to the baseline; the error text is the
/// classified `ApiError` one (SEC-11).
pub(super) async fn viewer_for(
    state: &AppState,
    principal: Option<&Principal>,
) -> Result<Viewer, Value> {
    // `BI-19` B2 review note: nobody attached sees nothing (fail closed), not
    // the anonymous viewer that reads as the unowned dashboards.
    let Some(principal) = principal else {
        return Ok(Viewer::nothing());
    };
    crate::routes::dashboard_access::viewer_for(state, principal)
        .await
        .map_err(|err| json!({ "error": err.to_string() }))
}

/// [`viewer_for`] for the tools that write (create, update and delete a chart,
/// arrange a board, move a chart, the tab tools): the tool's own gate asks for
/// `dashboard:write`, so the starting point is the write one, exactly as for the
/// write routes (`dashboard_access::viewer_for_write`, `BI-19` B1 review R5).
/// A restricted dashboard stays out of reach for somebody the rules leave out.
pub(super) async fn viewer_for_write(
    state: &AppState,
    principal: Option<&Principal>,
) -> Result<Viewer, Value> {
    let Some(principal) = principal else {
        return Ok(Viewer::nothing());
    };
    crate::routes::dashboard_access::viewer_for_write(state, principal)
        .await
        .map_err(|err| json!({ "error": err.to_string() }))
}

/// The write tools' starting point (`BI-19` B2 review B2-1): the caller's
/// [`Scope`](crate::routes::dashboard_access::Scope), the one the console's
/// write routes use, so the assistant is judged by the same levels. With nobody
/// attached there is nobody to judge and nothing is written (fail closed).
pub(super) async fn scope_for_write(
    state: &AppState,
    principal: Option<&Principal>,
) -> Result<crate::routes::dashboard_access::Scope, Value> {
    let Some(principal) = principal else {
        return Err(json!({
            "error": "the dashboard tools change a dashboard as the signed-in person, and there is none."
        }));
    };
    crate::routes::dashboard_access::scope_for_write(state, principal)
        .await
        .map_err(|err| json!({ "error": err.to_string() }))
}

/// Who may make a `create_chart` or `update_chart` call and on which
/// dashboards: a signed-in person ([`scope_for_write`]), then the map the call
/// names ([`maps::require_known_map`]), then the level checks of the console's
/// write routes (`specs_create`/`specs_update`), `edit` on the dashboard the
/// chart is on and on the one it moves to (`BI-19` B2 review B2-1). The one
/// copy of these checks: the dispatcher runs it before the save and
/// [`check_draft`] runs it before the draft, so a draft is refused with the
/// reply the save gives (`PR #391 review`, `PR #402 review`).
///
/// Returns the caller's [`Scope`](crate::routes::dashboard_access::Scope),
/// the viewer both the save and the draft read as.
async fn require_chart_call_allowed(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Result<crate::routes::dashboard_access::Scope, Value> {
    let scope = scope_for_write(state, principal).await?;
    // `AI-6`: a chart call that names a map that does not exist is refused
    // here, so the draft and the save refuse it with one sentence. It runs
    // after the writer check (nobody without `dashboard:write` learns which
    // maps exist) and before the board checks (the map catalogue needs no
    // dashboard).
    maps::require_known_map(state, name, args).await?;
    if name == "create_chart" {
        require_edit_on(state, &scope, &arg_str(args, "board")).await?;
    } else {
        require_edit_on_chart(state, &scope, &arg_str(args, "id"), Some(args)).await?;
    }
    Ok(scope)
}

/// `edit` on the dashboard `board` (blank = the built-in one), the answer of a
/// refusal as a tool error: the same 404 sentence for one the caller cannot see
/// and the 403 sentence for one they see and may not edit (`BI-19` B2 review
/// B2-1; a narrowing-only change, the tool did no such check before).
pub(super) async fn require_edit_on(
    state: &AppState,
    scope: &crate::routes::dashboard_access::Scope,
    board: &str,
) -> Result<(), Value> {
    crate::routes::dashboard_access::require_board_level(
        state,
        scope,
        board,
        lakehouse_bi::access::Level::Edit,
    )
    .await
    .map(|_| ())
    .map_err(|err| json!({ "error": err.to_string() }))
}

/// `edit` on the dashboard chart `id` is on and, when the call names another
/// `board` (`update_chart`), on that one too. A chart the caller cannot edit is
/// answered as the tool's own gate does (`require_editable_chart`).
pub(super) async fn require_edit_on_chart(
    state: &AppState,
    scope: &crate::routes::dashboard_access::Scope,
    id: &str,
    target: Option<&Map<String, Value>>,
) -> Result<(), Value> {
    if id.is_empty() {
        // The tool answers "id is required" itself.
        return Ok(());
    }
    let board = crate::routes::dashboard_organise::require_editable_chart(
        &state.clickhouse,
        id,
        &scope.viewer,
    )
    .await
    .map_err(dashboards::refused)?;
    require_edit_on(state, scope, &board).await?;
    if let Some(target) = target {
        let to = arg_str(target, "board");
        if !to.trim().is_empty() {
            require_edit_on(state, scope, &to).await?;
        }
    }
    Ok(())
}

/// Checks the requester of a `WriteHigh` call before an approval is created
/// (`BI-26` slice 2 D4, `DEC-8`). The gate answers "pending approval", and
/// this runs before `request_approval`: an error goes to the model as the tool
/// result and no approval row is written, so a call that could never run does
/// not ask a person to approve it.
///
/// The two embed writes run the same checks their console routes do: the one
/// manage guard (`public_links::managed_board`, `BI-26` review fix R6), the
/// configured `EMBED_SECRET` (`SEC-12` D4), and the argument validation the
/// route makes (`embed::check_filter_modes`; `embed_access::validate_origins`).
/// Every other tool is a no-op here: its own tool body checks what it needs.
///
/// # Errors
///
/// The tool-result JSON the model sees, as the routes' own sentence.
pub(crate) async fn check_before_approval(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Result<(), Value> {
    match name {
        "enable_embed" => {
            let board = managed_embed_board(state, principal, args).await?;
            embed_configured(state)?;
            let sites = arg_string_list(args, "sites")?;
            let sites = lakehouse_bi::embed_access::validate_origins(&sites)
                .map_err(|err| json!({ "error": err }))?;
            // D6: the limit is on the board's list after the addition, so a
            // call that would be the twenty-first site is refused here too,
            // before an approval exists.
            merged_origins(&board, &sites)
                .map(|_| ())
                .map_err(|err| json!({ "error": err }))
        }
        "set_embed_filter_modes" => {
            let board = managed_embed_board(state, principal, args).await?;
            embed_configured(state)?;
            let modes = arg_filter_modes(args)?;
            crate::routes::embed::check_filter_modes(&board, &modes)
                .map_err(|err| json!({ "error": err.to_string() }))
        }
        _ => Ok(()),
    }
}

/// The dashboard an embed tool acts on: the caller must manage it, by the one
/// guard the embed settings route uses (`BI-26` review fix R6). A call with no
/// signed-in person is refused: the embed settings are managed as somebody.
async fn managed_embed_board(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Result<lakehouse_bi::store::Board, Value> {
    let Some(principal) = principal else {
        return Err(json!({
            "error": "the embed tools change a dashboard as the signed-in person, and there is none."
        }));
    };
    let board = arg_str(args, "board");
    crate::routes::public_links::managed_board(state, principal, Some(&board))
        .await
        .map_err(|err| json!({ "error": err.to_string() }))
}

/// `SEC-12` D4: signed embedding cannot work without `EMBED_SECRET`, so every
/// embed tool refuses with the one sentence the routes use, before any
/// approval exists (`BI-26` slice 2 D5).
fn embed_configured(state: &AppState) -> Result<(), Value> {
    if state.config.embed_secret.as_deref().is_some() {
        Ok(())
    } else {
        Err(json!({ "error": crate::routes::embed::EMBEDDING_NOT_CONFIGURED }))
    }
}

/// `BI-26` slice 2 D6: the sites a board would have after adding `sites` to
/// the ones it lists now. A site already listed is not an error, and more than
/// [`lakehouse_bi::embed_access::MAX_ORIGINS`] is refused with the validator's
/// own sentence. Read at execution from the board the manage guard resolved,
/// so two approvals in a row cannot lose an earlier site; the pre-approval hook
/// uses the same function so a call that could never run asks no one.
fn merged_origins(
    board: &lakehouse_bi::store::Board,
    sites: &[String],
) -> Result<Vec<String>, String> {
    let mut merged = board.embed_access.origins.clone();
    for site in sites {
        if !merged.contains(site) {
            merged.push(site.clone());
        }
    }
    if merged.len() > lakehouse_bi::embed_access::MAX_ORIGINS {
        return Err(format!(
            "at most {} sites can be allowed per dashboard.",
            lakehouse_bi::embed_access::MAX_ORIGINS
        ));
    }
    Ok(merged)
}

/// `args[key]` as a list of strings; a missing key, a non-list or a non-string
/// entry is refused rather than read as empty (`BI-26` slice 2 D4).
fn arg_string_list(args: &Map<String, Value>, key: &str) -> Result<Vec<String>, Value> {
    let Some(Value::Array(entries)) = args.get(key) else {
        return Err(json!({ "error": format!("{key} must be a list.") }));
    };
    entries
        .iter()
        .map(|entry| entry.as_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()
        .ok_or_else(|| json!({ "error": format!("{key} must be a list of text entries.") }))
}

/// `args["modes"]` as the map the settings route writes: a column to one of
/// `locked`, `editable`, `hidden`.
fn arg_filter_modes(
    args: &Map<String, Value>,
) -> Result<std::collections::BTreeMap<String, lakehouse_bi::embed_access::FilterMode>, Value> {
    let Some(modes) = args.get("modes") else {
        return Err(json!({ "error": "modes is required." }));
    };
    serde_json::from_value(modes.clone()).map_err(
        |_| json!({ "error": "modes must be {column: \"locked\"|\"editable\"|\"hidden\"}" }),
    )
}

/// Checks a chart call before the gate offers it as a draft, and
/// adds what the draft holds to it. `draft` is the gate's `needs_confirmation`
/// answer; a call that is not a chart call is returned as it came. A chart
/// call that builds comes back as the draft with a `definition` key, the
/// digest of what the save would store (for an update the stored chart merged
/// with the call): the card reads `args`, so the key changes nothing the
/// person sees. The error otherwise, so a draft that the save would refuse (a
/// mart or column that does not exist, a dashboard the caller may not edit, a
/// SQL source they cannot see) is refused rather than offered. Writes nothing.
///
/// The draft runs the save's checks, from the same code and in the save's
/// order: [`require_chart_call_allowed`] here, then the preparation the save
/// shares with `dashboards::check_chart_draft`. It reads as `principal`,
/// through the same [`scope_for_write`] the save uses, so a chart, a dashboard
/// or a SQL source the caller cannot see answers what the save answers and
/// nothing of it. With nobody attached the call is refused (fail closed), for
/// a create as for an update.
///
/// When the reporting settings cannot be read, this returns the same error
/// the dispatch arm returns, instead of skipping the check.
///
/// `confirmed` is dropped during parse because `ChartInput` and its flattened `TableFields` lack
/// `deny_unknown_fields` (`Compare` has it; adding it here would refuse drafts the Confirm path accepts).
///
/// # Errors
///
/// The reply that replaces the draft when the call cannot be built.
pub(crate) async fn check_draft(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
    mut draft: Value,
) -> Result<Value, Value> {
    if name != "create_chart" && name != "update_chart" {
        return Ok(draft);
    }
    // Built and judged before anything else is read, as the dispatcher does,
    // so a caller the save would refuse is refused here with the same reply.
    let scope = require_chart_call_allowed(state, principal, name, args).await?;
    let Ok(time) = crate::routes::settings::time_context(state).await else {
        return Err(json!({ "error": "The reporting settings could not be read." }));
    };
    let id = (name == "update_chart").then(|| arg_str(args, "id"));
    let definition =
        dashboards::check_chart_draft(&state.clickhouse, &scope.viewer, args, id, &time).await?;
    draft["definition"] = Value::String(definition);
    Ok(draft)
}

/// Reads `args[key]` as a string, matching `String(args[key] ?? "")`: a
/// missing/`null` value becomes `""`, a non-string `Value` is
/// stringified rather than rejected (mirroring `TypeScript`'s implicit
/// coercion at this same boundary in `ai-tools.ts`).
fn arg_str(args: &Map<String, Value>, key: &str) -> String {
    args.get(key)
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn dispatch_test_state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    /// D3.5: every registered tool reaches a real dispatch arm (never the
    /// `unreachable!` in [`run_tool`]'s `other` arm, and never the
    /// "unknown tool" branch), and an unregistered name gets exactly
    /// that refusal shape. See `tests/fixtures/tool_schemas.json` and the
    /// characterization test in `super::super::tests` for the pre-refactor
    /// baseline this must keep matching.
    #[tokio::test]
    async fn run_tool_dispatches_every_registered_tool_and_refuses_unknown() {
        let state = dispatch_test_state();
        for spec in registry::TOOLS {
            let result = run_tool(&state, None, spec.name, &Map::new()).await;
            if let Some(err) = result.get("error").and_then(Value::as_str) {
                assert!(
                    !err.starts_with("unknown tool"),
                    "{} unexpectedly hit the unknown-tool dispatch arm: {err}",
                    spec.name
                );
            }
        }
        let unknown = run_tool(&state, None, "not_a_real_tool", &Map::new()).await;
        assert_eq!(unknown, json!({ "error": "unknown tool: not_a_real_tool" }));
    }

    /// The draft check of a chart update reads the stored chart as the
    /// caller, so with nobody attached there is nobody to read it as: it is
    /// refused with the sentence the save gives, before any store is asked
    /// (the state here has no reachable `ClickHouse`).
    #[tokio::test]
    async fn the_draft_check_of_an_update_chart_with_nobody_signed_in_is_refused() {
        let state = dispatch_test_state();
        let mut args = Map::new();
        args.insert("id".to_owned(), json!("c_1"));
        let draft = json!({ "needs_confirmation": true });
        let refused = check_draft(&state, None, "update_chart", &args, draft.clone())
            .await
            .expect_err("refused");
        assert_eq!(
            refused,
            scope_for_write(&state, None).await.map(|_| ()).unwrap_err(),
            "the draft and the save give one sentence"
        );
        assert!(
            refused["error"]
                .as_str()
                .is_some_and(|message| message.contains("there is none")),
            "{refused}"
        );
        // A call that is not a chart call is not checked at all.
        assert_eq!(
            check_draft(&state, None, "list_boards", &args, draft.clone()).await,
            Ok(draft)
        );
    }

    /// BI-26 slice 2 T4 (D4): the pre-approval hook is a no-op for every tool
    /// that is not one of the two embed writes, so adding it to the dispatch
    /// sites changes no other call. A tool that is not an embed write is never
    /// touched by the hook, even with no signed-in person.
    #[tokio::test]
    async fn check_before_approval_is_a_no_op_for_every_other_tool() {
        let state = dispatch_test_state();
        for spec in registry::TOOLS {
            if spec.name == "enable_embed" || spec.name == "set_embed_filter_modes" {
                continue;
            }
            assert!(
                check_before_approval(&state, None, spec.name, &Map::new())
                    .await
                    .is_ok(),
                "{} was checked by the pre-approval hook",
                spec.name
            );
        }
    }

    /// BI-26 slice 2 T4 (D4): both embed writes are refused with nobody
    /// signed in, before any approval can be made, because the embed settings
    /// are managed as somebody (`public_links::managed_board`).
    #[tokio::test]
    async fn check_before_approval_refuses_an_embed_write_with_nobody_signed_in() {
        let state = dispatch_test_state();
        for name in ["enable_embed", "set_embed_filter_modes"] {
            let error = check_before_approval(&state, None, name, &Map::new())
                .await
                .expect_err("refused");
            assert!(
                error["error"]
                    .as_str()
                    .is_some_and(|message| message.contains("there is none")),
                "{name}: {error}"
            );
        }
    }

    /// A stand-in `ClickHouse` that holds dashboards `b1` and `b2` (unfiled,
    /// unowned) and the chart `c1` on `b1`, enough for the write tools' level
    /// checks, which run before anything is written.
    #[derive(Default)]
    struct TwoBoards {
        /// The owner of `b2`: empty for a dashboard nobody owns, otherwise
        /// somebody's personal space.
        b2_owner: &'static str,
    }

    impl wiremock::Respond for TwoBoards {
        fn respond(&self, request: &wiremock::Request) -> wiremock::ResponseTemplate {
            let sql = String::from_utf8_lossy(&request.body).into_owned();
            let rows: Vec<Value> = if sql.contains("FROM console.bi_chart FINAL WHERE id = 'c1'") {
                vec![json!({ "is_deleted": 0, "board": "b1" })]
            } else if sql.contains("SELECT is_deleted, owner_id FROM console.bi_board FINAL") {
                // `board_row_state`: the dashboard exists, live, unowned.
                ["b1", "b2"]
                    .iter()
                    .filter(|id| sql.contains(&format!("id = '{id}'")))
                    .map(|id| json!({ "is_deleted": 0, "owner_id": self.owner_of(id) }))
                    .collect()
            } else if sql.contains("FROM console.bi_board FINAL") {
                ["b1", "b2"]
                    .iter()
                    .filter(|id| sql.contains(&format!("id='{id}'")))
                    .map(|id| {
                        json!({ "id": id, "name": id, "folder_id": "", "owner_id": self.owner_of(id) })
                    })
                    .collect()
            } else {
                Vec::new()
            };
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({ "meta": [], "data": rows, "rows": rows.len() }))
        }
    }

    impl TwoBoards {
        fn owner_of(&self, board: &str) -> &'static str {
            if board == "b2" { self.b2_owner } else { "" }
        }
    }

    fn state_over_two_boards(uri: &str) -> AppState {
        let env = HashMap::from([("CH_URL".to_owned(), uri.to_owned())]);
        AppState::new(Config::from_map(&env).unwrap())
    }

    fn caller_with(permissions: &str) -> Principal {
        Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::from_u128(9)),
            tenant_ids: Vec::new(),
            display_name: "Rina".to_owned(),
            permissions: lakehouse_auth::PermissionSet::parse(permissions),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn five_write_calls() -> Vec<(&'static str, Map<String, Value>)> {
        let args = |v: Value| v.as_object().cloned().unwrap();
        vec![
            ("create_chart", args(json!({ "board": "b1", "title": "T" }))),
            ("update_chart", args(json!({ "id": "c1", "board": "b2" }))),
            ("delete_chart", args(json!({ "id": "c1" }))),
            ("move_chart", args(json!({ "id": "c1", "board": "b2" }))),
            ("format_chart", args(json!({ "chart": "c1" }))),
        ]
    }

    /// `BI-19` B2 review B2-1: the assistant's five chart-writing tools are
    /// judged by the same level as the console's routes. A caller whose
    /// starting point is `view` (`dashboard:read` only) reaches the gate in a
    /// unit test (the gate is not in play when `run_tool` is called) and is
    /// refused by every tool with the 403 sentence, and nothing is written.
    #[tokio::test]
    async fn a_caller_at_view_level_is_refused_by_all_five_chart_write_tools() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let reader = caller_with("dashboard:read");
        for (tool, args) in five_write_calls() {
            let reply = run_tool(&state, Some(&reader), tool, &args).await;
            let error = reply["error"].as_str().unwrap_or_default();
            assert!(
                error.contains("edit on this dashboard"),
                "{tool} should be refused for a view caller, got {reply}"
            );
        }
    }

    /// The other half: at `manage` (a `dashboard:write` holder) the level check
    /// lets the call through, so no tool answers with the level or the
    /// not-found sentence of the check (a later step may still fail on the
    /// stand-in, which holds no marts).
    #[tokio::test]
    async fn a_caller_at_manage_level_passes_the_level_check_of_the_five_tools() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let writer = caller_with("dashboard:read, dashboard:write");
        for (tool, args) in five_write_calls() {
            let reply = run_tool(&state, Some(&writer), tool, &args).await;
            let error = reply["error"].as_str().unwrap_or_default();
            assert!(
                !error.contains("edit on this dashboard") && !error.contains("dashboard not found"),
                "{tool} should pass the level check for a writer, got {reply}"
            );
        }
    }

    /// With nobody attached the write tools change nothing (and the read
    /// tools' fallback sees nothing).
    #[tokio::test]
    async fn the_chart_write_tools_change_nothing_with_nobody_attached() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        for (tool, args) in five_write_calls() {
            let reply = run_tool(&state, None, tool, &args).await;
            assert!(reply["error"].is_string(), "{tool}: {reply}");
            assert!(reply.get("created").is_none() && reply.get("updated").is_none());
        }
    }

    /// The draft of a chart call gets the save's answer, whole: the same value
    /// `run_tool` gives, and no draft (`PR #391 review`, `PR #402 review`).
    async fn assert_the_draft_is_refused_as_the_save_is(
        state: &AppState,
        principal: Option<&Principal>,
        tool: &str,
        args: &Map<String, Value>,
    ) -> Value {
        let saved = run_tool(state, principal, tool, args).await;
        assert!(saved["error"].is_string(), "{tool}: {saved}");
        let draft = json!({ "needs_confirmation": true });
        let refused = check_draft(state, principal, tool, args, draft)
            .await
            .expect_err("the draft is refused");
        assert_eq!(refused, saved, "the draft and the save give one reply");
        saved
    }

    /// A chart the caller sees and may not edit gets the 403 sentence from the
    /// draft check, not a Confirm card that the save then refuses.
    #[tokio::test]
    async fn the_draft_of_an_update_of_a_chart_the_caller_may_not_edit_is_refused_as_the_save_is() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let reader = caller_with("dashboard:read");
        let args = json!({
            "id": "c1", "title": "T", "kind": "bar", "mart": "mart_x",
            "dimension": "province", "measures": ["visitors"],
        })
        .as_object()
        .cloned()
        .unwrap();
        let saved = assert_the_draft_is_refused_as_the_save_is(
            &state,
            Some(&reader),
            "update_chart",
            &args,
        )
        .await;
        assert!(
            saved["error"]
                .as_str()
                .is_some_and(|e| e.contains("edit on this dashboard")),
            "{saved}"
        );
    }

    /// An update that moves the chart onto a dashboard the caller cannot reach
    /// is refused by the draft check as the save refuses it.
    #[tokio::test]
    async fn the_draft_of_an_update_that_moves_the_chart_onto_a_foreign_dashboard_is_refused_as_the_save_is()
     {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards {
                b2_owner: "somebody-else",
            })
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let writer = caller_with("dashboard:read, dashboard:write");
        let args = json!({
            "id": "c1", "title": "T", "kind": "bar", "mart": "mart_x",
            "dimension": "province", "measures": ["visitors"], "board": "b2",
        })
        .as_object()
        .cloned()
        .unwrap();
        let saved = assert_the_draft_is_refused_as_the_save_is(
            &state,
            Some(&writer),
            "update_chart",
            &args,
        )
        .await;
        assert_eq!(saved["error"], json!("dashboard not found."), "{saved}");
    }

    /// A create draft with nobody attached is refused with the save's
    /// sentence; it used to read as nobody and carry on.
    #[tokio::test]
    async fn the_draft_of_a_create_with_nobody_signed_in_is_refused_as_the_save_is() {
        let state = dispatch_test_state();
        let args = json!({ "title": "T", "kind": "bar", "mart": "mart_x" })
            .as_object()
            .cloned()
            .unwrap();
        let saved =
            assert_the_draft_is_refused_as_the_save_is(&state, None, "create_chart", &args).await;
        assert!(
            saved["error"]
                .as_str()
                .is_some_and(|message| message.contains("there is none")),
            "{saved}"
        );
    }

    /// `AI-6`: a chart call that names a map that does not exist is refused by
    /// the draft check and by the save, with one sentence that names the maps
    /// the model may use.
    #[tokio::test]
    async fn the_draft_of_a_create_on_a_map_that_does_not_exist_is_refused_as_the_save_is() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let writer = caller_with("dashboard:read, dashboard:write");
        let args = json!({
            "board": "b1", "title": "T", "kind": "geomap", "map": "indonesia",
        })
        .as_object()
        .cloned()
        .unwrap();
        let saved = assert_the_draft_is_refused_as_the_save_is(
            &state,
            Some(&writer),
            "create_chart",
            &args,
        )
        .await;
        assert_eq!(
            saved["error"],
            json!(
                "map 'indonesia' does not exist. Use one of: dki-jakarta, id-provinces, id-regencies, world, adm1-<iso3> for a country's provinces, or an uploaded map from list_maps."
            ),
            "{saved}"
        );
    }

    /// A chart on a map that exists is not refused by the map check: the
    /// built-in ids and a country's map pass it.
    #[tokio::test]
    async fn a_chart_on_a_map_that_exists_is_not_refused_by_the_map_check() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let writer = caller_with("dashboard:read, dashboard:write");
        for map in ["id-provinces", "world", "adm1-usa"] {
            let args = json!({
                "board": "b1", "title": "T", "kind": "geomap", "map": map,
            })
            .as_object()
            .cloned()
            .unwrap();
            let saved = run_tool(&state, Some(&writer), "create_chart", &args).await;
            assert!(
                !saved["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("does not exist. Use one of:"),
                "{map} may be named: {saved}"
            );
        }
    }

    /// An update that names no map — none at all, a `null` one or a blank one
    /// — keeps the stored one, so the map check has nothing to refuse.
    #[tokio::test]
    async fn an_update_that_names_no_map_is_not_refused_by_the_map_check() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(TwoBoards::default())
            .mount(&server)
            .await;
        let state = state_over_two_boards(&server.uri());
        let writer = caller_with("dashboard:read, dashboard:write");
        for map in [None, Some(json!(null)), Some(json!("   "))] {
            let mut args = json!({ "id": "c1", "title": "T", "kind": "bar" })
                .as_object()
                .cloned()
                .unwrap();
            if let Some(map) = map.as_ref() {
                args.insert("map".to_owned(), map.clone());
            }
            let saved = run_tool(&state, Some(&writer), "update_chart", &args).await;
            assert!(
                !saved["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("does not exist. Use one of:"),
                "{map:?}: {saved}"
            );
        }
    }

    // PR review fix (SHOULD-FIX): `AI_ASK_BACK=false` only hid `ask_user` from
    // the tool list, so a model that named it anyway still ran it.
    #[tokio::test]
    async fn ask_user_runs_with_the_switch_on_and_is_refused_with_it_off() {
        let mut args = Map::new();
        args.insert("term".to_owned(), json!("hotel"));
        args.insert("question".to_owned(), json!("Which one?"));
        args.insert("options".to_owned(), json!(["hotel_a", "hotel_b"]));
        let state_with = |switch: &str| {
            let env = HashMap::from([("AI_ASK_BACK".to_owned(), switch.to_owned())]);
            AppState::new(Config::from_map(&env).unwrap())
        };

        let on = run_tool(&state_with("true"), None, "ask_user", &args).await;
        assert_eq!(on["asked"], json!(true), "{on}");

        let off = run_tool(&state_with("false"), None, "ask_user", &args).await;
        assert_eq!(off, json!({ "error": "asking the user is switched off" }));
        // Only `ask_user` is behind the switch.
        let other = run_tool(&state_with("false"), None, "list_datasets", &Map::new()).await;
        assert_ne!(other["error"], json!("asking the user is switched off"));
    }

    #[test]
    fn arg_str_defaults_missing_or_null_to_empty_string() {
        let mut args = Map::new();
        args.insert("a".to_owned(), Value::Null);
        assert_eq!(arg_str(&args, "a"), "");
        assert_eq!(arg_str(&args, "missing"), "");
    }

    #[test]
    fn arg_str_returns_string_value_verbatim() {
        let mut args = Map::new();
        args.insert("a".to_owned(), Value::String("hi".to_owned()));
        assert_eq!(arg_str(&args, "a"), "hi");
    }
}
