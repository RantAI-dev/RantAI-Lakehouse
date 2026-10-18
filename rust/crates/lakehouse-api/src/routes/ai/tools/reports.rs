//! Scheduled-report tools (`AI-10`, schedule half; `RPT-1` plan D10):
//! `create_report_subscription`, `list_report_subscriptions`,
//! `update_report_subscription` and `delete_report_subscription`.
//!
//! They call the same rules as the console's routes
//! ([`crate::routes::reports::validate_input`]): the download right, a
//! dashboard the person can see, charts that belong to it, outside addresses
//! only on a domain an administrator listed, at most 50 recipients.
//!
//! # Who needs to approve (feature decision 10; product owner, 2026-10-17)
//!
//! - A subscription that sends only to the person asking is `WriteLow`: the
//!   console's Confirm button, no approval.
//! - A call that names any other recipient is `WriteHigh` for that call
//!   ([`crate::routes::ai::gate::effective_risk`]): it waits in Approvals until
//!   a person with `agent:approve` decides. So is every delete.
//!
//! # The report belongs to the requester, not to the approver
//!
//! An approved action runs as the approver (`decide_approval` hands
//! `run_tool` the approver). A report made as the approver would carry the
//! approver's masking and row filters to the recipients. So before the
//! approval is created, [`approval_args`] checks the requester's rights and
//! stamps their id into the stored arguments as `ownerId`; when the approved
//! call runs, the tool reads `ownerId`, makes the subscription theirs and
//! validates it as them (their rights today). The approver is also checked
//! when it runs: `dashboard:read` (the registry's permission, checked by
//! `decide_approval`), the download right, and the dashboard being visible to
//! them. A model cannot name an owner: `ownerId` is stripped from every
//! call's arguments ([`crate::routes::ai::gate::strip_confirmed`]) and only
//! ever written here, server side, into an approval's stored arguments.

use lakehouse_auth::Principal;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_store::report_subscriptions::{self as subscriptions, Subscription};
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::arg_str;
use crate::routes::dashboard_organise::viewer_of;
use crate::routes::reports::{self, SubscriptionInput, schedule_json, validate_input};
use crate::state::AppState;

const SIGN_IN: &str = "Sign in to manage scheduled reports.";
const NOT_YOURS: &str = "Only the person who set this report up can change it from the assistant; an administrator can use the dashboard's Subscriptions.";
const NO_SUCH: &str = "That subscription does not exist.";
const OWNER_GONE: &str = "The person who asked for this report is no longer active.";
const FAILED: &str = "The request could not be completed.";

/// The four tools.
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "create_report_subscription"
            | "list_report_subscriptions"
            | "update_report_subscription"
            | "delete_report_subscription"
    )
}

/// The sentence for a refused call. Every `ApiError` `validate_input` and the
/// store produce carries one of the routes' fixed sentences; anything else
/// (a store failure under a reference) reads as [`FAILED`].
fn sentence(err: &ApiError) -> String {
    match err {
        ApiError::BadRequest(s)
        | ApiError::NotFound(s)
        | ApiError::Unavailable(s)
        | ApiError::Unprocessable(s)
        | ApiError::PermissionDenied(s) => s.clone(),
        _ => FAILED.to_owned(),
    }
}

fn refusal(err: &ApiError) -> Value {
    json!({ "error": sentence(err) })
}

/// Dispatch one of the four tools.
pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    let Some(caller) = principal else {
        return json!({ "error": SIGN_IN });
    };
    let owner = match acting_owner(state, caller, args).await {
        Ok(owner) => owner,
        Err(error) => return error,
    };
    // After an approval the approver releases the requester's report: the
    // dashboard must be one they can see themselves.
    if owner.id != caller.id
        && let Some(board) = board_of(state, name, args).await
    {
        let seen = store::get_board(&state.clickhouse, &board, &viewer_of(caller)).await;
        if !matches!(&seen, Ok(Some(_))) && board != store::DEFAULT_BOARD_ID {
            return json!({ "error": "You cannot see that dashboard, so you cannot approve a report of it." });
        }
    }
    match name {
        "create_report_subscription" => create(state, &owner, args).await,
        "list_report_subscriptions" => list(state, &owner, args).await,
        "update_report_subscription" => update(state, &owner, args).await,
        "delete_report_subscription" => delete(state, &owner, args).await,
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}

/// The dashboard a call is about: the `board` argument of a create, the board of
/// the subscription an update or delete names.
async fn board_of(state: &AppState, name: &str, args: &Map<String, Value>) -> Option<String> {
    if name == "create_report_subscription" {
        return Some(arg_str(args, "board")).filter(|b| !b.trim().is_empty());
    }
    let id = arg_str(args, "id").parse::<Uuid>().ok()?;
    subscriptions::get(state.pg.as_deref()?, id)
        .await
        .ok()
        .flatten()
        .map(|s| s.board)
}

/// The person the call acts for. Normally the caller. After an approval, the
/// stamped requester (`ownerId`), loaded as they are now; the approver must then
/// also be able to download.
async fn acting_owner(
    state: &AppState,
    caller: &Principal,
    args: &Map<String, Value>,
) -> Result<Principal, Value> {
    let Some(stamped) = args.get("ownerId").and_then(Value::as_str) else {
        return Ok(caller.clone());
    };
    if stamped == caller.id.uuid().to_string() {
        return Ok(caller.clone());
    }
    // Run after an approval, by someone else: the approver must be allowed to
    // download too (they are releasing data to the recipients).
    crate::routes::settings::require_download(state, caller)
        .await
        .map_err(|e| refusal(&e))?;
    match crate::routes::render::principal_of(state, stamped).await {
        Ok(Some(owner)) => Ok(owner),
        Ok(None) => Err(json!({ "error": OWNER_GONE })),
        Err(_) => Err(json!({ "error": FAILED })),
    }
}

fn pool(state: &AppState) -> Result<&lakehouse_store::PgPool, Value> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| json!({ "error": "The database is unavailable." }))
}

/// A subscription as the model reads it: no addresses of people other than
/// the names and addresses the owner chose.
async fn brief(state: &AppState, sub: &Subscription) -> Value {
    let recipients = match state.pg.as_deref() {
        Some(pool) => subscriptions::recipients(pool, sub.id)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    json!({
        "id": sub.id,
        "board": sub.board,
        "schedule": sub.schedule().map_or(Value::Null, schedule_json),
        "timeZone": sub.time_zone,
        "attachments": sub.attachments,
        "skipWhenEmpty": sub.skip_when_empty,
        "paused": sub.paused,
        "pausedReason": sub.paused_reason,
        "nextRunAt": sub.next_run_at.to_string(),
        "recipients": recipients.iter().map(|r| match (&r.user_id, &r.email) {
            (Some(id), _) => json!({ "userId": id, "name": r.user_name, "email": r.user_email }),
            (None, email) => json!({ "email": email }),
        }).collect::<Vec<_>>(),
    })
}

fn email_note(state: &AppState) -> Option<&'static str> {
    state
        .config
        .smtp_host
        .is_none()
        .then_some("Email is not set up on this deployment, so nothing is sent until it is.")
}

// ── Arguments to the console's body ──────────────────────────────────────

/// The subscription body the console's rules check, from the tool's flat
/// arguments. Recipients: the person themselves unless `sendToMe` is false,
/// plus the others named in `recipients`. `existing` supplies whatever an
/// update leaves out.
fn input_from_args(
    args: &Map<String, Value>,
    me: Uuid,
    existing: Option<&Subscription>,
) -> Result<SubscriptionInput, String> {
    let given = |key: &str| args.contains_key(key);
    let schedule = match (args.get("schedule"), existing) {
        (Some(s), _) => s.clone(),
        (None, Some(e)) => e
            .schedule()
            .map(schedule_json)
            .map_err(|_| FAILED.to_owned())?,
        (None, None) => return Err("schedule is required.".to_owned()),
    };
    let attachments = if existing.is_none()
        || ["pdf", "charts", "paper", "orientation"]
            .iter()
            .any(|k| given(k))
    {
        let base = existing.map_or(&Value::Null, |e| &e.attachments);
        let pdf_now = match args.get("pdf").and_then(Value::as_bool) {
            Some(on) => on,
            None => existing.is_none_or(|_| !base["pdf"].is_null()),
        };
        let pdf = pdf_now.then(|| {
            json!({
                "paper": args.get("paper").and_then(Value::as_str)
                    .or_else(|| base["pdf"]["paper"].as_str()).unwrap_or("a4"),
                "orientation": args.get("orientation").and_then(Value::as_str)
                    .or_else(|| base["pdf"]["orientation"].as_str()).unwrap_or("portrait"),
            })
        });
        let charts = args
            .get("charts")
            .cloned()
            .unwrap_or_else(|| base.get("charts").cloned().unwrap_or_else(|| json!([])));
        json!({ "pdf": pdf, "charts": charts })
    } else {
        existing.map_or(Value::Null, |e| e.attachments.clone())
    };
    let mut recipients: Vec<Value> = Vec::new();
    let recipients_given = given("recipients") || given("sendToMe");
    // An update that names no recipients keeps the stored ones; the caller
    // re-reads them (`update`).
    if recipients_given || existing.is_none() {
        if args
            .get("sendToMe")
            .and_then(Value::as_bool)
            .unwrap_or(true)
        {
            recipients.push(json!({ "userId": me.to_string() }));
        }
        if let Some(others) = args.get("recipients").and_then(Value::as_array) {
            recipients.extend(others.iter().cloned());
        }
    }
    let pick =
        |key: &str, stored: Option<Value>| -> Option<Value> { args.get(key).cloned().or(stored) };
    let body = json!({
        "schedule": schedule,
        "timeZone": pick("timeZone", existing.map(|e| json!(e.time_zone))),
        "attachments": attachments,
        "filters": pick("filters", existing.and_then(|e| e.filters.clone())),
        "params": pick("params", existing.and_then(|e| e.params.clone())),
        "grain": pick("grain", existing.and_then(|e| e.grain.clone().map(Value::String))),
        "skipWhenEmpty": args.get("skipWhenEmpty").and_then(Value::as_bool)
            .or_else(|| existing.map(|e| e.skip_when_empty)).unwrap_or(false),
        "recipients": recipients,
    });
    serde_json::from_value(body).map_err(|_| {
        "The arguments do not describe a report: check schedule, recipients and charts.".to_owned()
    })
}

// ── The tools ────────────────────────────────────────────────────────────

async fn create(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let board = arg_str(args, "board");
    if board.trim().is_empty() {
        return json!({ "error": "board is required." });
    }
    let input = match input_from_args(args, owner.id.uuid(), None) {
        Ok(input) => input,
        Err(e) => return json!({ "error": e }),
    };
    let new = match validate_input(state, owner, board.trim(), &input).await {
        Ok(new) => new,
        Err(e) => return refusal(&e),
    };
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match subscriptions::create(pool, &new).await {
        Ok(made) => {
            let mut out = json!({ "created": true, "subscription": brief(state, &made).await });
            if let Some(note) = email_note(state) {
                out["warning"] = json!(note);
            }
            out
        }
        Err(_) => json!({ "error": FAILED }),
    }
}

async fn list(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let board = arg_str(args, "board");
    let Ok(rows) = subscriptions::list_for_owner(pool, owner.id.uuid()).await else {
        return json!({ "error": FAILED });
    };
    let mut items = Vec::new();
    for sub in rows.iter().filter(|s| board.is_empty() || s.board == board) {
        items.push(brief(state, sub).await);
    }
    json!({ "items": items })
}

/// The caller's subscription by `id` argument.
async fn own(
    state: &AppState,
    owner: &Principal,
    args: &Map<String, Value>,
) -> Result<Subscription, Value> {
    let id = arg_str(args, "id")
        .parse::<Uuid>()
        .map_err(|_| json!({ "error": NO_SUCH }))?;
    let sub = subscriptions::get(pool(state)?, id)
        .await
        .map_err(|_| json!({ "error": FAILED }))?
        .ok_or_else(|| json!({ "error": NO_SUCH }))?;
    if sub.owner_id == owner.id.uuid() || owner.has("settings:write") {
        Ok(sub)
    } else {
        Err(json!({ "error": NOT_YOURS }))
    }
}

async fn update(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let sub = match own(state, owner, args).await {
        Ok(sub) => sub,
        Err(e) => return e,
    };
    if sub.owner_id != owner.id.uuid() {
        return json!({ "error": NOT_YOURS });
    }
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    let mut input = match input_from_args(args, owner.id.uuid(), Some(&sub)) {
        Ok(input) => input,
        Err(e) => return json!({ "error": e }),
    };
    // Recipients the call did not touch stay as stored.
    if !args.contains_key("recipients") && !args.contains_key("sendToMe") {
        input.recipients = match subscriptions::recipients(pool, sub.id).await {
            Ok(rows) => rows
                .iter()
                .filter_map(|r| match (&r.user_id, &r.email) {
                    (Some(id), _) => Some(reports::RecipientInput {
                        user_id: Some(id.to_string()),
                        email: None,
                    }),
                    (None, Some(email)) => Some(reports::RecipientInput {
                        user_id: None,
                        email: Some(email.clone()),
                    }),
                    (None, None) => None,
                })
                .collect(),
            Err(_) => return json!({ "error": FAILED }),
        };
    }
    input.paused = args.get("paused").and_then(Value::as_bool);
    let new = match validate_input(state, owner, &sub.board, &input).await {
        Ok(new) => new,
        Err(e) => return refusal(&e),
    };
    match reports::save_update(pool, &sub, &new, input.paused).await {
        Ok(saved) => json!({ "updated": true, "subscription": brief(state, &saved).await }),
        Err(e) => refusal(&e),
    }
}

async fn delete(state: &AppState, owner: &Principal, args: &Map<String, Value>) -> Value {
    let sub = match own(state, owner, args).await {
        Ok(sub) => sub,
        Err(e) => return e,
    };
    let pool = match pool(state) {
        Ok(p) => p,
        Err(e) => return e,
    };
    match subscriptions::delete(pool, sub.id).await {
        Ok(_) => json!({ "deleted": true, "id": sub.id }),
        Err(_) => json!({ "error": FAILED }),
    }
}

// ── Before an approval exists ────────────────────────────────────────────

/// Whether a call to this tool is stamped and checked before it waits for
/// approval.
pub(in crate::routes) fn needs_stamp(name: &str) -> bool {
    matches!(
        name,
        "create_report_subscription" | "update_report_subscription" | "delete_report_subscription"
    )
}

/// The arguments to store with an approval. For the report tools: the
/// requester's rights are checked now (so nobody is asked to approve what the
/// requester could not do), and their id is written into the arguments as
/// `ownerId`. Any other tool's arguments come back unchanged.
///
/// # Errors
///
/// The refusal to show instead of creating an approval.
pub(in crate::routes) async fn approval_args(
    state: &AppState,
    requester: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Result<Map<String, Value>, Value> {
    if !needs_stamp(name) {
        return Ok(args.clone());
    }
    let Some(requester) = requester else {
        return Err(json!({ "error": SIGN_IN }));
    };
    let me = requester.id.uuid();
    // What the requester asks for must be something they could do: the same
    // checks the tool makes when it runs, made now, as them.
    match name {
        "create_report_subscription" => {
            let board = arg_str(args, "board");
            let input = input_from_args(args, me, None).map_err(|e| json!({ "error": e }))?;
            validate_input(state, requester, board.trim(), &input)
                .await
                .map_err(|e| refusal(&e))?;
        }
        "update_report_subscription" => {
            let sub = own(state, requester, args).await?;
            if sub.owner_id != me {
                return Err(json!({ "error": NOT_YOURS }));
            }
            let input = input_from_args(args, me, Some(&sub)).map_err(|e| json!({ "error": e }))?;
            validate_input(state, requester, &sub.board, &input)
                .await
                .map_err(|e| refusal(&e))?;
        }
        _ => {
            own(state, requester, args).await?;
        }
    }
    let mut stamped = args.clone();
    stamped.insert("ownerId".to_owned(), json!(me.to_string()));
    Ok(stamped)
}
