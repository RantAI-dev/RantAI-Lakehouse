//! The dashboard-sharing tools (`AI-11`): make a public link only
//! with a human's approval, end one on a chat confirm, and say who can see a
//! dashboard.
//!
//! Two rules shape this module, the same two the embed tools run on. The
//! first is the manage guard: every tool here is judged as the signed-in
//! person through the one guard the console routes use
//! (`public_links::managed_board`), never through a `dashboard:write` gate —
//! a person who manages the dashboard through a rule alone holds no global
//! permission. The second is that the assistant never touches who a
//! dashboard is shared with by RULE: nothing here (or anywhere in the tools
//! modules) reaches the console's access-write handler — the Share > People
//! page owns that, and a guard test pins the absence.
//!
//! The writes go through the console's own handlers
//! (`public_links::{put_link, delete_link}`), so the route's manage guard,
//! the personal-space and switch checks, and the `dashboard.public_link_*`
//! audit rows apply unchanged; the route's own live-link check is
//! authoritative at execution. The link acts as the tenant of the person who
//! approves it (`SEC-50`): the tool call sends no `X-Tenant` header, so
//! `tenant_scope::resolve` uses the caller's first tenant, the tenant a
//! console save by the same person would use.
//!
//! The tool result carries the link's PATH (`/public/dashboard/<token>`) and
//! the token, honestly: there is no server-side public base URL to build the
//! full address from — the console's Share dialog adds the origin in the
//! browser (`public-link-panel.tsx`), so the tool says where the copyable
//! address lives instead of inventing one.

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::api_result_to_value;
use crate::state::AppState;

/// `AI-11`: the refusal when nobody other than the requester could
/// approve and run the link, so no approval row is written.
pub(super) const NO_OTHER_APPROVER: &str = "Only another manager of this dashboard can approve this, and it has none; make the link yourself in Share.";

/// The tool's own bound for `days` (`AI-11` decision 1): the route allows one
/// hour to a year, the assistant may ask for 1 to 30 days. Its own sentence,
/// because the route's expiry sentence would name bounds the schema never
/// offered.
const DAYS_REFUSED: &str = "The link can be made for 1 to 30 days.";

/// The default expiry, in days (`AI-11` decision 1).
const DEFAULT_DAYS: i64 = 30;

/// The result's note about the address, the same fact for every link: the
/// full URL is the console's, because only the browser knows the origin.
const URL_NOTE: &str = "Share has the copyable address: it is this console's origin plus the path below. The link is live until it expires or someone ends it.";

/// Whether `name` is one of these tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "create_public_link" | "revoke_public_link" | "list_dashboard_access"
    )
}

pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    match name {
        "create_public_link" => create_public_link(state, principal, args).await,
        "revoke_public_link" => revoke_public_link(state, principal, args).await,
        "list_dashboard_access" => list_dashboard_access(state, principal, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The dashboard a sharing tool acts on, as the signed-in person. The same
/// guard the embed tools run (`super::managed_embed_board`, which calls
/// `public_links::managed_board`); its sentence names "the embed tools", so
/// this module carries its own copy of the guard with words that name these
/// tools — the underlying route guard is shared, not duplicated.
///
/// # Errors
///
/// The tool-result JSON the model sees: the no-caller sentence, or the
/// route guard's refusal as `api_error_reply` maps it.
async fn sharing_board(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Result<lakehouse_bi::store::Board, Value> {
    let Some(principal) = principal else {
        return Err(json!({
            "error": "the sharing tools change a dashboard as the signed-in person, and there is none."
        }));
    };
    let board = super::arg_str(args, "board");
    super::api_error_reply(
        crate::routes::public_links::managed_board(state, principal, Some(&board)).await,
    )
}

/// `days` as the tool reads it: absent means the default; a value outside
/// 1 to 30 is refused (`AI-11` decision 1). A non-integer is refused the
/// same way, never truncated.
///
/// # Errors
///
/// The tool-result JSON the model sees: `DAYS_REFUSED` for a `days` that
/// is not an integer or sits outside 1 to 30.
fn requested_days(args: &Map<String, Value>) -> Result<i64, Value> {
    let Some(raw) = args.get("days") else {
        return Ok(DEFAULT_DAYS);
    };
    let days = raw
        .as_i64()
        .ok_or_else(|| json!({ "error": DAYS_REFUSED }))?;
    if (1..=30).contains(&days) {
        Ok(days)
    } else {
        Err(json!({ "error": DAYS_REFUSED }))
    }
}

/// The expiry the tool asks the route for: `days` from now, RFC 3339.
fn expiry_in_days(days: i64) -> String {
    let at = OffsetDateTime::now_utc() + time::Duration::days(days);
    at.format(&Rfc3339).unwrap_or_default()
}

/// The pre-approval checks of `create_public_link` (`AI-11` decision 1): a
/// call that could never run — a non-manager, an expiry outside the bound,
/// public links switched off, a personal board — is refused HERE, before any
/// approval row is written (`tools::check_before_approval`). The manage
/// guard is the route's own (`public_links::managed_board`); the switch and
/// personal-space checks mirror `put_link`'s, with the route's own
/// sentences, so the refusal the model sees is the refusal the console
/// would give.
///
/// # Errors
///
/// The tool-result JSON the model sees, as the routes' own sentence.
pub(super) async fn check_before_create(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Result<(), Value> {
    requested_days(args)?;
    let board = sharing_board(state, principal, args).await?;
    if board.owner_id.is_some() {
        return Err(json!({ "error": lakehouse_bi::store::PERSONAL_NOT_SHAREABLE }));
    }
    let enabled = super::api_error_reply(crate::routes::settings::format_settings(state).await)?
        .public_links_enabled;
    if !enabled {
        return Err(json!({ "error": crate::routes::public_links::SWITCHED_OFF }));
    }
    super::require_another_approver(
        state,
        principal,
        "create_public_link",
        args,
        NO_OTHER_APPROVER,
    )
    .await
}

/// `create_public_link {board, days?}`: make the dashboard's public link, or
/// move the live one's expiry, through `public_links::put_link`. With no live
/// link the route makes a new one (a new token); with one, the link keeps its
/// token and only the expiry moves. No `password` argument exists: the body
/// the tool sends omits the key entirely, because `PutBody` reads `null` as
/// "remove the password" and `deny_unknown_fields` refuses a bad key — the
/// assistant never sets or lifts a link's password.
async fn create_public_link(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "create_public_link changes a dashboard as the signed-in person, and there is none." });
    };
    let days = match requested_days(args) {
        Ok(days) => days,
        Err(error) => return error,
    };
    let board = match sharing_board(state, Some(principal), args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    // No `X-Tenant` header: the link acts as the approving person's first
    // tenant (`SEC-50`; `tenant_scope::resolve` resolves exactly that when
    // the header is absent), the same tenant a console save by them uses.
    let body = json!({ "board": board.id, "expiresAt": expiry_in_days(days) });
    let result = api_result_to_value(
        crate::routes::public_links::put_link(
            State(state.clone()),
            axum::Extension(principal.clone()),
            HeaderMap::new(),
            Bytes::from(serde_json::to_vec(&body).unwrap_or_default()),
        )
        .await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    let link = result.get("link").cloned().unwrap_or(Value::Null);
    let token = link
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    json!({
        "board": board.id,
        "url": format!("/public/dashboard/{token}"),
        "token": token,
        "expiresAt": link.get("expiresAt").cloned().unwrap_or(Value::Null),
        "hasPassword": link.get("hasPassword").cloned().unwrap_or(Value::Null),
        "note": URL_NOTE,
    })
}

/// `revoke_public_link {board}`: end the dashboard's public link at once
/// through `public_links::delete_link` — the route the Share dialog's switch
/// calls. Ending narrows access, so the gate has already asked for the
/// chat's Confirm; nothing here waits in Approvals. `ended: false` says so
/// honestly when there was no link to end.
async fn revoke_public_link(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "revoke_public_link changes a dashboard as the signed-in person, and there is none." });
    };
    let board = match sharing_board(state, Some(principal), args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    let Ok(query) = serde_json::from_value::<crate::routes::public_links::BoardQuery>(json!({
        "board": board.id,
    })) else {
        return json!({ "error": "The dashboard could not be read." });
    };
    let result = api_result_to_value(
        crate::routes::public_links::delete_link(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Query(query),
        )
        .await,
    )
    .await;
    if result.get("error").is_some() {
        return result;
    }
    json!({
        "board": board.id,
        "ended": result.get("ended").and_then(Value::as_bool).unwrap_or(false),
        "note": "The address stops working at once. Making a link again makes a new address.",
    })
}

/// `list_dashboard_access {board}`: who can see the dashboard, as the Share >
/// People tab shows it, through `dashboard_access::get_access` (the rules,
/// the caller's own level, the marks) plus the link's liveness through
/// `public_links::get_link`. The answer never carries the link's token or a
/// password: the token is a secret an outsider needs, so the tool names the
/// link's expiry and whether a password is set, nothing more.
async fn list_dashboard_access(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "list_dashboard_access reads a dashboard as the signed-in person, and there is none." });
    };
    let board = match sharing_board(state, Some(principal), args).await {
        Ok(board) => board,
        Err(error) => return error,
    };
    let Ok(access_query) = serde_json::from_value::<crate::routes::dashboard_access::AccessQuery>(
        json!({ "kind": "board", "id": board.id }),
    ) else {
        return json!({ "error": "The dashboard could not be read." });
    };
    let mut answer = api_result_to_value(
        crate::routes::dashboard_access::get_access(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Query(access_query),
        )
        .await,
    )
    .await;
    if answer.get("error").is_some() {
        return answer;
    }
    let Ok(link_query) = serde_json::from_value::<crate::routes::public_links::BoardQuery>(json!({
        "board": board.id,
    })) else {
        return json!({ "error": "The dashboard could not be read." });
    };
    let link = api_result_to_value(
        crate::routes::public_links::get_link(
            State(state.clone()),
            axum::Extension(principal.clone()),
            Query(link_query),
        )
        .await,
    )
    .await;
    // Live means it works: the administrator's switch is on AND the link's
    // expiry has not passed. Anything else is "no link" to a visitor.
    let live = link.get("enabled").and_then(Value::as_bool) == Some(true)
        && link
            .get("link")
            .and_then(Value::as_object)
            .is_some_and(|l| l.get("expired").and_then(Value::as_bool) == Some(false));
    answer["publicLink"] = if live {
        let l = link.get("link").and_then(Value::as_object);
        json!({
            "expiresAt": l.and_then(|m| m.get("expiresAt")).cloned().unwrap_or(Value::Null),
            "hasPassword": l.and_then(|m| m.get("hasPassword")).cloned().unwrap_or(Value::Null),
            "expired": l.and_then(|m| m.get("expired")).cloned().unwrap_or(Value::Null),
        })
    } else {
        Value::Null
    };
    // The embed state (`BI-26`): whether the dashboard can be framed on an
    // allowed site — part of who can see it, and never a secret.
    answer["embedding"] = json!(board.embed_enabled == Some(true));
    answer
}
